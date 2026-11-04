use super::requests::FetchRequests;
use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        command::{git_command_output, git_ref_listing},
        pool_install::present_commits,
        request_refs::{RequestViewBases, attach_visible_request_refs, request_ref_head},
    },
};
use scope_domain::requests::{Request, canonical_request_ref};
use std::{collections::BTreeSet, process::Command};

pub(super) async fn attach_refs(
    folder: &GitRepoHandle,
    selection: FetchRequests,
    bases: RequestViewBases,
    main_oid: &str,
) -> Result<Vec<Request>, ApiError> {
    let restoring = folder.share()?;
    let (selection, present) = blocking::run(move || {
        let folder = restoring;
        let FetchRequests { requests, retained } = selection;
        let mut present = present_commits(
            &folder,
            requests.iter().map(|request| request.head_oid.as_str())
                .chain(retained.iter().map(String::as_str)),
        )?;
        for request in requests.iter().filter(|request| request.pushed_at_unix.is_none()) {
            if present.contains(&request.head_oid) {
                continue;
            }
            match attach_visible_request_refs(std::slice::from_ref(request), &folder, &bases) {
                Ok(()) => {
                    if request_ref_head(&folder, &canonical_request_ref(&request.name))?.as_deref()
                        == Some(request.head_oid.as_str()) {
                        present.insert(request.head_oid.clone());

                    }
                }
                Err(error) => tracing::warn!(request_id = request.id, request_name = request.name,
                    error = %error.operator_diagnostic(), "omitting unreadable request from Git fetch"),
            }
        }
        Ok((FetchRequests { requests, retained }, present))
    }).await?;
    let folder = folder.share()?;
    let main_oid = main_oid.to_string();
    blocking::run(move || {
        let FetchRequests { requests, retained } = selection;
        let attached = requests
            .into_iter()
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
