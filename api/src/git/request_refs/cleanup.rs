use super::{
    acquire_request_ref_update_lock_async, locks::acquire_request_ref_store_lock, request_ref_head,
};
use crate::{
    error::ApiError,
    git::{import::run_git, storage::request_ref_store_repo_path},
    state::AppState,
};
use scope_domain::{repository::RepositoryIncarnation, requests::canonical_request_ref};

/// Cleans only the deleted request's local ref. The update lock spans the
/// metadata check and conditional Git deletion, fencing replacement requests.
pub(crate) async fn cleanup_deleted_request_ref(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_name: &str,
    expected_head_oid: &str,
) -> Result<(), ApiError> {
    scope_domain::requests::validate_request_name(request_name).map_err(ApiError::bad_request)?;
    let request_ref = canonical_request_ref(request_name);
    let update_lock =
        acquire_request_ref_update_lock_async(state, incarnation, &request_ref).await?;
    if state
        .metadata
        .cleanup()
        .request_ref_is_live(incarnation, request_name)
        .await?
    {
        return Ok(());
    }
    let state = state.clone();
    let incarnation = incarnation.clone();
    let expected_head_oid = expected_head_oid.to_string();
    crate::git::blocking::run(move || {
        let _update_lock = update_lock;
        let _store_lock = acquire_request_ref_store_lock(&state, &incarnation)?;
        let store_repo = request_ref_store_repo_path(&state, &incarnation);
        if !store_repo.exists() {
            return Ok(());
        }
        if request_ref_head(&store_repo, &request_ref)?.as_deref()
            == Some(expected_head_oid.as_str())
        {
            run_git(
                Some(&store_repo),
                &["update-ref", "-d", &request_ref, &expected_head_oid],
                "deleting obsolete request ref",
            )?;
        }
        Ok(())
    })
    .await
}
