use super::requests::FetchRequests;
use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        command::{git_command_output, git_ref_listing, run_git},
        pool_install::{present_commits, write_shelf},
        request_refs::{RequestViewBases, attach_readable_request_refs, request_ref_head},
    },
    state::AppState,
};
use scope_domain::requests::{Request, canonical_request_ref};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Command,
};

pub(super) async fn attach_refs(
    state: &AppState,
    folder: &GitRepoHandle,
    selection: FetchRequests,
    bases: RequestViewBases,
    main_oid: &str,
    shelves: BTreeMap<(String, String), PathBuf>,
) -> Result<Vec<Request>, ApiError> {
    let restore_state = state.clone();
    let restoring = folder.share()?;
    let (selection, present, restored) = blocking::run(move || {
        let state = restore_state;
        let folder = restoring;
        let mut restored = Vec::new();
        let FetchRequests { requests, unadvertised, retained } = selection;
        let mut present = present_commits(
            &folder,
            requests.iter().map(|request| request.head_oid.as_str())
                .chain(retained.iter().map(String::as_str)),
        )?;
        let snapshots_first = requests
            .iter()
            .filter(|request| request.git_snapshot.is_some())
            .chain(requests.iter().filter(|request| request.git_snapshot.is_none()));
        for request in snapshots_first {
            if present.contains(&request.head_oid) {
                continue;
            }
            match attach_readable_request_refs(&state, std::slice::from_ref(request), &folder, &bases) {
                Ok(unreadable) if unreadable.is_empty() => {
                    if request_ref_head(&folder, &canonical_request_ref(&request.name))?.as_deref()
                        == Some(request.head_oid.as_str()) {
                        present.insert(request.head_oid.clone());
                        if let Some(shelf) = shelves.get(&(request.id.clone(), request.head_oid.clone())) {
                            restored.push((request.head_oid.clone(), shelf.clone()));
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(request_id = request.id, request_name = request.name,
                    error = %error.operator_diagnostic(), "omitting unreadable request from Git fetch"),
            }
        }
        Ok((FetchRequests { requests, unadvertised, retained }, present, restored))
    }).await?;
    for (head, shelf) in restored {
        let target = shelf.clone();
        let source = folder.share()?;
        let result = write_shelf(&shelf, move || {
            run_git(
                Some(&target),
                &[
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    source.to_string_lossy().as_ref(),
                    &format!("+{head}:refs/scope/restored/{head}"),
                ],
                "retaining restored request objects",
            )
        })
        .await;
        if let Err(error) = result {
            tracing::warn!(error = %error.operator_diagnostic(), "restored request objects were not retained");
        }
    }
    let folder = folder.share()?;
    let main_oid = main_oid.to_string();
    blocking::run(move || {
        let FetchRequests {
            requests,
            unadvertised,
            retained,
        } = selection;
        let attached = requests
            .into_iter()
            .filter(|request| !unadvertised.contains(&request.id))
            .filter(|request| {
                if present.contains(&request.head_oid) {
                    true
                } else {
                    tracing::warn!(
                        request_id = request.id,
                        request_name = request.name,
                        "omitting unavailable request from Git fetch"
                    );
                    false
                }
            })
            .collect::<Vec<_>>();
        let mut refs = format!("update refs/heads/main {main_oid}\n");
        let mut named = BTreeSet::new();
        for request in &attached {
            let name = canonical_request_ref(&request.name);
            if named.insert(name.clone()) {
                refs.push_str(&format!("update {name} {}\n", request.head_oid));
            }
        }
        for (name, _) in git_ref_listing(&folder, &["refs/heads"], "reading restored fetch refs")? {
            if name != "refs/heads/main" && !named.contains(&name) {
                refs.push_str(&format!("delete {name}\n"));
            }
        }
        for oid in retained {
            if present.contains(&oid) {
                refs.push_str(&format!("update refs/scope/fetch/{oid} {oid}\n"));
            }
        }
        git_command_output(
            Command::new("git")
                .arg("-C")
                .arg(folder.as_ref())
                .args(["update-ref", "--stdin"]),
            Some(refs.as_bytes()),
        )?;
        Ok(attached)
    })
    .await
}
