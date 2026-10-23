use super::git::{GitPoolPack, GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCompactionPlan {
    selected_packs: [GitPoolPack; 2],
}

impl GitCompactionPlan {
    pub fn select(
        packs: &[GitPoolPack],
        max_source_bytes: u64,
    ) -> Result<Option<Self>, GitCompactionError> {
        let spans = validate_main_chain(packs)?;

        let Some(pair_start) = spans.windows(2).position(|pair| {
            pair[0].count == pair[1].count
                && packs[pair[0].index]
                    .storage
                    .stored_bytes()
                    .checked_add(packs[pair[1].index].storage.stored_bytes())
                    .is_some_and(|total| total <= max_source_bytes)
        }) else {
            return Ok(None);
        };

        Ok(Some(Self {
            selected_packs: [packs[pair_start].clone(), packs[pair_start + 1].clone()],
        }))
    }

    pub fn selected_packs(&self) -> &[GitPoolPack; 2] {
        &self.selected_packs
    }

    pub fn replacement(&self, segment: GitSegmentRef) -> Result<GitPoolPack, GitCompactionError> {
        let [first, last] = &self.selected_packs;
        let replacement = GitPoolPack {
            id: segment.segment_id.clone(),
            owner: GitPoolPackOwner::Main {
                first_sequence: main_range(first)?.first,
                last_sequence: main_range(last)?.last,
            },
            head_oid: last.head_oid.clone(),
            prerequisites: first.prerequisites.clone(),
            storage: GitPoolPackStorage::Segment(segment),
        };
        self.validate_replacement(&replacement)?;
        Ok(replacement)
    }

    pub fn validate_replacement(
        &self,
        replacement: &GitPoolPack,
    ) -> Result<(), GitCompactionError> {
        let [first, last] = &self.selected_packs;
        let selected = [main_range(first)?, main_range(last)?];
        validate_link(first, &selected[0], last, &selected[1])?;
        if selected[0].count()? != selected[1].count()? {
            return Err(GitCompactionError::SelectedSizesDiffer);
        }
        let covered = main_range(replacement)?;
        if covered.first != selected[0].first
            || covered.last != selected[1].last
            || replacement.prerequisites != first.prerequisites
            || replacement.head_oid != last.head_oid
            || replacement
                .storage
                .segment()
                .map(|segment| &segment.segment_id)
                != Some(&replacement.id)
        {
            return Err(GitCompactionError::ReplacementBoundaryMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct MainRange {
    first: u64,
    last: u64,
}

impl MainRange {
    fn count(self) -> Result<u64, GitCompactionError> {
        self.last
            .checked_sub(self.first)
            .and_then(|distance| distance.checked_add(1))
            .ok_or(GitCompactionError::InvalidRange {
                first_sequence: self.first,
                last_sequence: self.last,
            })
    }
}

struct ChainEntry {
    index: usize,
    count: u64,
}

fn main_range(pack: &GitPoolPack) -> Result<MainRange, GitCompactionError> {
    pack.owner
        .main_sequences()
        .map(|(first, last)| MainRange { first, last })
        .ok_or_else(|| GitCompactionError::NotMainPack {
            pack_id: pack.id.clone(),
        })
}

fn validate_main_chain(packs: &[GitPoolPack]) -> Result<Vec<ChainEntry>, GitCompactionError> {
    let mut entries = Vec::with_capacity(packs.len());
    let mut previous: Option<(&GitPoolPack, MainRange)> = None;
    for (index, pack) in packs.iter().enumerate() {
        let range = main_range(pack)?;
        let count = range.count()?;
        if !count.is_power_of_two() {
            return Err(GitCompactionError::NonGeometricCoverage {
                first_sequence: range.first,
                last_sequence: range.last,
                sequence_count: count,
            });
        }
        match previous {
            Some((previous_pack, previous_range)) => {
                validate_link(previous_pack, &previous_range, pack, &range)?
            }
            None if range.first != 1 || !pack.prerequisites.is_empty() => {
                return Err(GitCompactionError::InvalidStart {
                    first_sequence: range.first,
                });
            }
            None => {}
        }
        entries.push(ChainEntry { index, count });
        previous = Some((pack, range));
    }
    Ok(entries)
}

fn validate_link(
    previous: &GitPoolPack,
    previous_range: &MainRange,
    next: &GitPoolPack,
    next_range: &MainRange,
) -> Result<(), GitCompactionError> {
    if previous_range.last.checked_add(1) != Some(next_range.first) {
        return Err(GitCompactionError::NonContiguous {
            previous_last_sequence: previous_range.last,
            next_first_sequence: next_range.first,
        });
    }
    if next.prerequisites != [previous.head_oid.clone()] {
        return Err(GitCompactionError::DisconnectedHistory {
            previous_head_oid: previous.head_oid.clone(),
            next_prerequisites: next.prerequisites.clone(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GitCompactionError {
    #[error("Git pool pack {pack_id} is not a main pack")]
    NotMainPack { pack_id: String },
    #[error("Git main pack range {first_sequence}..{last_sequence} is invalid")]
    InvalidRange {
        first_sequence: u64,
        last_sequence: u64,
    },
    #[error(
        "Git main pack {first_sequence}..{last_sequence} covers {sequence_count} pushes instead of a power of two"
    )]
    NonGeometricCoverage {
        first_sequence: u64,
        last_sequence: u64,
        sequence_count: u64,
    },
    #[error(
        "Git main packs must start at sequence 1 without prerequisites, found {first_sequence}"
    )]
    InvalidStart { first_sequence: u64 },
    #[error(
        "Git main packs have a gap or overlap between sequences {previous_last_sequence} and {next_first_sequence}"
    )]
    NonContiguous {
        previous_last_sequence: u64,
        next_first_sequence: u64,
    },
    #[error(
        "Git main pack history is disconnected: previous head {previous_head_oid}, next prerequisites {next_prerequisites:?}"
    )]
    DisconnectedHistory {
        previous_head_oid: String,
        next_prerequisites: Vec<String>,
    },
    #[error("Git compaction replacement must cover exactly the selected main packs")]
    ReplacementBoundaryMismatch,
    #[error("Git compaction requires adjacent main packs covering the same number of pushes")]
    SelectedSizesDiffer,
}

#[cfg(test)]
mod tests;
