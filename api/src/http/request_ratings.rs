use super::requests::{actor_request, repo_metadata_and_access, visible_request};
use super::responses::request_actor_summary_response;
use crate::after_commit::AfterCommit;
use crate::{
    auth::viewer::Viewer, error::ApiError, persistence::unix_now, repo_events::RepoChangeReason,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreateRequestRatingRequest, RequestRatingResponse, RequestRatingsResponse,
};
use scope_domain::{
    account::UserAccount,
    requests::{CreateRequestRatingInput, RequestRating, eligible_rating_subject_user_id},
};
use scope_product_analytics::ProductEvent;
use std::collections::BTreeMap;

pub(crate) async fn list_request_ratings(
    State(state): State<AppState>,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    viewer: Viewer,
) -> Result<Json<RequestRatingsResponse>, ApiError> {
    let (repo, viewer_user_id) =
        repo_metadata_and_access(&state, &viewer, &owner, &repo_name).await?;
    let (request, _) =
        visible_request(&state, &repo, viewer_user_id.as_deref(), &request_id).await?;
    ratings_response(&state, &request, viewer_user_id.as_deref()).await
}

pub(crate) async fn create_request_rating(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(payload): Json<CreateRequestRatingRequest>,
) -> Result<Json<RequestRatingResponse>, ApiError> {
    let (user, repo, request) =
        actor_request(&state, &headers, &owner, &repo_name, &request_id).await?;
    let rating = state
        .metadata
        .requests()
        .create_request_rating(CreateRequestRatingInput {
            id: crate::persistence_ids::generate_prefixed_id("request_rating")?,
            request_id: request.id.clone(),
            actor_user_id: user.id.clone(),
            score: payload.score,
            reason: payload.reason,
            now_unix: unix_now()?,
        })
        .await?;
    state.after_commit([
        AfterCommit::Analytics(ProductEvent::request_rated(
            &user.id,
            &repo.record.incarnation_id,
            &request.id,
            request.view,
            rating.score,
        )),
        AfterCommit::request_summary(&repo.incarnation(), RepoChangeReason::RequestRated),
    ]);
    let users = state
        .metadata
        .auth()
        .users_by_ids([rating.rater_user_id.clone(), rating.subject_user_id.clone()])
        .await?;
    Ok(Json(rating_response(rating, &users)?))
}

async fn ratings_response(
    state: &AppState,
    request: &scope_domain::requests::Request,
    viewer_user_id: Option<&str>,
) -> Result<Json<RequestRatingsResponse>, ApiError> {
    let ratings = state
        .metadata
        .requests()
        .request_ratings(&request.id)
        .await?;
    let eligible_subject_id = viewer_user_id.and_then(|viewer_user_id| {
        eligible_rating_subject_user_id(request, viewer_user_id, &ratings).map(str::to_string)
    });
    let user_ids = ratings
        .iter()
        .flat_map(|rating| [rating.rater_user_id.clone(), rating.subject_user_id.clone()])
        .chain(eligible_subject_id.iter().cloned());
    let users = state.metadata.auth().users_by_ids(user_ids).await?;
    let ratings = ratings
        .into_iter()
        .map(|rating| rating_response(rating, &users))
        .collect::<Result<Vec<_>, _>>()?;
    let eligible_subject = eligible_subject_id
        .as_deref()
        .map(|user_id| request_actor_summary_response(user_id, &users))
        .transpose()?;
    Ok(Json(RequestRatingsResponse {
        ratings,
        eligible_subject,
    }))
}

fn rating_response(
    rating: RequestRating,
    users: &BTreeMap<String, UserAccount>,
) -> Result<RequestRatingResponse, ApiError> {
    Ok(RequestRatingResponse {
        rater: request_actor_summary_response(&rating.rater_user_id, users)?,
        subject: request_actor_summary_response(&rating.subject_user_id, users)?,
        id: rating.id,
        request_id: rating.request_id,
        score: rating.score,
        reason: rating.reason,
        created_at_unix: rating.created_at_unix,
    })
}
