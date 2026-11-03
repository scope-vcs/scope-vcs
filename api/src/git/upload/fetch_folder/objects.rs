use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        repository_git::RepositoryGit,
        request_refs::{
            pool::{
                PoolInstall, PoolRepos, install_request_pool_packs, request_object_repos,
                shares_objects,
            },
            view_pool::view_object_repo,
        },
    },
    state::AppState,
};
use scope_domain::{repo_config::RepoConfig, requests::Request, views::ViewId};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(super) struct FetchObjects {
    pub(super) alternates: Vec<PathBuf>,
    pub(super) leases: Vec<GitRepoHandle>,
    pub(super) shelves: BTreeMap<(String, String), PathBuf>,
}

pub(super) async fn install_objects(
    state: &AppState,
    git: &RepositoryGit,
    config: &RepoConfig,
    view: &ViewId,
    user: Option<&str>,
    base: &GitRepoHandle,
    requests: &[Request],
) -> Result<FetchObjects, ApiError> {
    let full = view == config.views.full();
    let mut alternates = Vec::new();
    let mut shelf_leases = Vec::new();
    let mut shelves = BTreeMap::new();
    if full {
        let shelf_state = state.clone();
        let incarnation = git.incarnation.clone();
        let generation = config.object_generation();
        let author = user.unwrap_or_default().to_string();
        let repos = blocking::run(move || {
            request_object_repos(&shelf_state, &incarnation, &generation, &author)
        })
        .await?;
        alternates.extend([
            base.as_ref().to_path_buf(),
            repos.shared.clone(),
            repos.authored.clone(),
        ]);
        install_request_pool_packs(
            state,
            &PoolInstall {
                incarnation: &git.incarnation,
                config,
                repos: PoolRepos::Full(&repos, user.unwrap_or_default(), base),
            },
            requests,
        )
        .await;
        for request in requests {
            let target = if shares_objects(request, config) {
                Some(&repos.shared)
            } else if user.is_some_and(|user| request.is_author(user)) {
                Some(&repos.authored)
            } else {
                None
            };
            if let Some(target) = target {
                shelves.insert(
                    (request.id.clone(), request.head_oid.clone()),
                    target.clone(),
                );
            }
        }
    }
    let request_views = requests
        .iter()
        .map(|request| request.view.clone())
        .chain((!full).then_some(view.clone()))
        .filter(|view| view != config.views.full())
        .collect::<BTreeSet<_>>();
    for request_view in request_views {
        let shelf = view_object_repo(
            state,
            &git.incarnation,
            config,
            &request_view,
            (!full).then_some(user.unwrap_or_default()),
        )
        .await?;
        let subset = requests
            .iter()
            .filter(|request| {
                request.view == request_view
                    && (full
                        || request.is_submitted()
                        || user.is_some_and(|user| request.is_author(user)))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !full {
            validate_isolated_shelf(&shelf)?;
        }
        install_request_pool_packs(
            state,
            &PoolInstall {
                incarnation: &git.incarnation,
                config,
                repos: PoolRepos::View(&request_view, &shelf),
            },
            &subset,
        )
        .await;
        for request in subset {
            shelves
                .entry((request.id, request.head_oid))
                .or_insert_with(|| shelf.as_ref().to_path_buf());
        }
        alternates.push(shelf.as_ref().to_path_buf());
        shelf_leases.push(shelf);
    }
    Ok(FetchObjects {
        alternates,
        leases: shelf_leases,
        shelves,
    })
}

fn validate_isolated_shelf(shelf: &Path) -> Result<(), ApiError> {
    let alternates =
        fs::read_to_string(shelf.join("objects/info/alternates")).map_err(ApiError::internal)?;
    let name = shelf
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once("-fetch-"))
        .map(|(name, _)| name)
        .ok_or_else(|| ApiError::internal_message("invalid restricted fetch shelf"))?;
    let expected = fs::canonicalize(
        shelf
            .with_file_name(format!("{name}-projection.git"))
            .join("objects"),
    )
    .map_err(ApiError::internal)?;
    if alternates.lines().collect::<Vec<_>>() != [expected.to_string_lossy().as_ref()]
        || expected.join("info/alternates").exists()
    {
        return Err(ApiError::internal_message(
            "restricted fetch shelf has an unauthorized alternate",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_isolated_shelf_passes_under_a_symlinked_data_directory() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        let objects = real.join("view-projection.git/objects");
        let info = real.join("view-fetch-reader.git/objects/info");
        fs::create_dir_all(&objects).unwrap();
        fs::create_dir_all(&info).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let stored = fs::canonicalize(&objects).unwrap();
        fs::write(info.join("alternates"), format!("{}\n", stored.display())).unwrap();
        validate_isolated_shelf(&link.join("view-fetch-reader.git")).unwrap();
        fs::write(info.join("alternates"), "/elsewhere/objects\n").unwrap();
        assert!(validate_isolated_shelf(&link.join("view-fetch-reader.git")).is_err());
    }
}
