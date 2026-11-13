mod walk;

use crate::{error::ApiError, runtime_budgets::RuntimeBudgets};
use gix::{ObjectId, objs::Kind, refs::TargetRef};
use std::{
    collections::BTreeSet,
    fmt::Display,
    path::Path as FsPath,
    time::{Duration, Instant},
};

pub(crate) use walk::MergeBases;

const MAX_TAG_DEPTH: usize = 32;
const MAX_TAG_BYTES: u64 = 1024 * 1024;

enum Peeled {
    Missing,
    Commit(ObjectId),
    Other,
}

pub(crate) struct GitReader {
    repo: gix::Repository,
    deadline: Instant,
    budget: Duration,
}

impl GitReader {
    pub(crate) fn open(path: &FsPath) -> Option<Self> {
        Self::open_with_budget(path, RuntimeBudgets::default_git_command_timeout())
    }

    fn open_with_budget(path: &FsPath, budget: Duration) -> Option<Self> {
        let deadline = Instant::now() + budget;
        let options =
            gix::open::Options::isolated().config_overrides(["core.useReplaceRefs=false"]);
        let repo = settled(gix::open_opts(path, options))?;
        if settled(repo.is_shallow())?
            || has_replacements(&repo)?
            || repo.git_dir().join("info").join("grafts").exists()
        {
            return None;
        }
        Some(Self {
            repo,
            deadline,
            budget,
        })
    }

    #[cfg(test)]
    fn open_with_budget_for_tests(path: &FsPath) -> Self {
        let repo = gix::open_opts(path, gix::open::Options::isolated()).unwrap();
        Self {
            repo,
            deadline: Instant::now() + RuntimeBudgets::default_git_command_timeout(),
            budget: RuntimeBudgets::default_git_command_timeout(),
        }
    }

    pub(crate) fn is_commit(&self, oid: &str) -> Option<bool> {
        let id = self.canonical_oid(oid)?;
        let header = settled(self.repo.try_find_header(id))?;
        Some(header.is_some_and(|header| header.kind() == Kind::Commit))
    }

    pub(crate) fn present_commits(
        &self,
        oids: &[&str],
        action: &str,
    ) -> Result<Option<BTreeSet<String>>, ApiError> {
        let mut present = BTreeSet::new();
        for oid in oids {
            self.within_deadline(action)?;
            let Some(is_commit) = self.is_commit(oid) else {
                return Ok(None);
            };
            if is_commit {
                present.insert((*oid).to_string());
            }
        }
        Ok(Some(present))
    }

    pub(crate) fn peels_to_commit(&self, oid: &str) -> Option<bool> {
        Some(match self.peel(self.canonical_oid(oid)?)? {
            Peeled::Commit(_) => true,
            Peeled::Missing | Peeled::Other => false,
        })
    }

    pub(crate) fn reference_target(&self, name: &str) -> Option<Option<String>> {
        Some(self.reference_id(name)?.map(|id| id.to_string()))
    }

    pub(crate) fn reference_commit(&self, name: &str) -> Option<String> {
        let id = self.reference_id(name)??;
        Some(self.peeled_commit(id)?.to_string())
    }

    pub(crate) fn references(
        &self,
        prefixes: &[&str],
        action: &str,
    ) -> Result<Option<Vec<(String, String)>>, ApiError> {
        if prefixes
            .iter()
            .any(|prefix| prefix.is_empty() || prefix.contains(['*', '?', '[', '\\']))
        {
            return Ok(None);
        }
        let Some(platform) = settled(self.repo.references()) else {
            return Ok(None);
        };
        let Some(all) = settled(platform.all()) else {
            return Ok(None);
        };
        let mut listed = Vec::new();
        for reference in all {
            self.within_deadline(action)?;
            let Some(mut reference) = settled(reference) else {
                return Ok(None);
            };
            let Ok(name) = std::str::from_utf8(reference.name().as_bstr()) else {
                return Ok(None);
            };
            let name = name.to_string();
            if !name.starts_with("refs/") || !matches_ref_prefix(&name, prefixes) {
                continue;
            }
            let id = match reference.target() {
                TargetRef::Object(id) => id.to_owned(),
                TargetRef::Symbolic(_) => match reference.follow_to_object() {
                    Ok(id) => id.detach(),
                    Err(error) if error.is_not_found() => continue,
                    Err(error) => {
                        tracing::debug!(%error, "Git reader deferred to git");
                        return Ok(None);
                    }
                },
            };
            listed.push((name, id.to_string()));
        }
        listed.sort();
        Ok(Some(listed))
    }

    pub(crate) fn is_ancestor(
        &self,
        ancestor: &str,
        descendant: &str,
        action: &str,
    ) -> Result<Option<bool>, ApiError> {
        let (Some(ancestor), Some(descendant)) = (
            self.revision_commit(ancestor),
            self.revision_commit(descendant),
        ) else {
            return self.unanswered(action);
        };
        Ok(walk::merge_bases(self, ancestor, descendant, action)?
            .map(|bases| bases.contains(&ancestor)))
    }

    pub(crate) fn merge_bases(
        &self,
        left: &str,
        right: &str,
        action: &str,
    ) -> Result<Option<MergeBases>, ApiError> {
        let (Some(left), Some(right)) = (self.revision_commit(left), self.revision_commit(right))
        else {
            return self.unanswered(action);
        };
        Ok(walk::merge_bases(self, left, right, action)?.map(MergeBases::from_bases))
    }

    fn within_deadline(&self, action: &str) -> Result<(), ApiError> {
        if Instant::now() >= self.deadline {
            return Err(self.timed_out(action));
        }
        Ok(())
    }

    fn unanswered<T>(&self, action: &str) -> Result<Option<T>, ApiError> {
        if Instant::now() >= self.deadline {
            return Err(self.timed_out(action));
        }
        Ok(None)
    }

    pub(super) fn timed_out(&self, action: &str) -> ApiError {
        ApiError::infrastructure_unavailable(format!(
            "failed {action}: Git reader timed out after {}ms",
            self.budget.as_millis()
        ))
    }

    fn revision_commit(&self, revision: &str) -> Option<ObjectId> {
        let id = if revision.starts_with("refs/") {
            self.reference_id(revision)??
        } else {
            self.canonical_oid(revision)?
        };
        self.peeled_commit(id)
    }

    fn reference_id(&self, name: &str) -> Option<Option<ObjectId>> {
        if !name.starts_with("refs/") {
            return None;
        }
        let Some(mut reference) = settled(self.repo.try_find_reference(name))? else {
            return Some(None);
        };
        Some(Some(settled(reference.follow_to_object())?.detach()))
    }

    fn peeled_commit(&self, id: ObjectId) -> Option<ObjectId> {
        match self.peel(id)? {
            Peeled::Commit(id) => Some(id),
            Peeled::Missing | Peeled::Other => None,
        }
    }

    fn peel(&self, mut id: ObjectId) -> Option<Peeled> {
        for _ in 0..MAX_TAG_DEPTH {
            if Instant::now() >= self.deadline {
                return None;
            }
            let Some(header) = settled(self.repo.try_find_header(id))? else {
                return Some(Peeled::Missing);
            };
            match header.kind() {
                Kind::Commit => return Some(Peeled::Commit(id)),
                Kind::Tree | Kind::Blob => return Some(Peeled::Other),
                Kind::Tag if header.size() > MAX_TAG_BYTES => return None,
                Kind::Tag => {
                    let tag = settled(settled(self.repo.try_find_object(id))??.try_into_tag())?;
                    id = settled(tag.target_id())?.detach();
                }
            }
        }
        None
    }

    fn canonical_oid(&self, oid: &str) -> Option<ObjectId> {
        let canonical = oid.len() == self.repo.object_hash().len_in_hex()
            && oid
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !canonical {
            return None;
        }
        settled(ObjectId::from_hex(oid.as_bytes()))
    }
}

fn has_replacements(repo: &gix::Repository) -> Option<bool> {
    let platform = settled(repo.references())?;
    let mut replacements = settled(platform.prefixed("refs/replace/"))?;
    Some(replacements.next().is_some())
}

fn matches_ref_prefix(name: &str, prefixes: &[&str]) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|prefix| {
            name.strip_prefix(prefix).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('/') || prefix.ends_with('/')
            })
        })
}

fn settled<T, E: Display>(result: Result<T, E>) -> Option<T> {
    result
        .inspect_err(|error| tracing::debug!(%error, "Git reader deferred to git"))
        .ok()
}

#[cfg(test)]
mod tests;
