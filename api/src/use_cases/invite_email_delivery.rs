//! Sends queued repository invite emails and records what happened.

use crate::{
    auth::tokens::generate_repository_invite_token,
    error::ApiError,
    http::origins::public_app_origin,
    invite_mailer::{InviteEmailMessage, InviteEmailOutcome},
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::repository_collaboration::publish_committed_mutation,
};
use scope_domain::{
    repo_invite_email::InviteEmailAttempt,
    repository::{Repository, collaboration::RepositoryInvite},
};
use scope_postgres::error::PostgresErrorKind;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(15);
const BATCH_SIZE: u64 = 20;

pub(crate) async fn deliver_due_invite_emails(
    state: &AppState,
    now: u64,
) -> Result<usize, ApiError> {
    let due = state
        .metadata
        .repositories()
        .due_repository_invite_email_ids(now, BATCH_SIZE)
        .await?;
    for email_id in &due {
        if let Err(error) = deliver_invite_email(state, email_id, now).await {
            tracing::warn!(
                %email_id,
                error = %error.operator_diagnostic(),
                "invite email delivery failed; it stays queued"
            );
        }
    }
    Ok(due.len())
}

async fn deliver_invite_email(state: &AppState, email_id: &str, now: u64) -> Result<(), ApiError> {
    let repositories = state.metadata.repositories();
    // The link is created here and never stored; the invite keeps its hash.
    let (secret, link_hash) = generate_repository_invite_token()?;
    let outcome = match repositories
        .issue_repository_invite_email_link(
            email_id,
            link_hash,
            now,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
    {
        Ok(issued) => {
            let delivery =
                publish_committed_mutation(state, issued, RepoChangeReason::InviteUpdated).await;
            let inviter = repositories
                .user(&delivery.invite.invited_by_user_id)
                .await?;
            let message = invite_email_message(
                email_id,
                &delivery.repo,
                &delivery.invite,
                &inviter.email,
                &secret,
                now,
            )?;
            state.invite_mailer.send(&message).await
        }
        // The invite was revoked, accepted, or expired while this was queued.
        Err(error) if error.kind == PostgresErrorKind::Conflict => InviteEmailOutcome {
            attempt: InviteEmailAttempt::Refused(error.message),
            provider_message_id: None,
        },
        Err(error) => return Err(error.into()),
    };
    if let Some(settled) = repositories
        .record_repository_invite_email_attempt(
            email_id,
            outcome.attempt,
            outcome.provider_message_id,
            now,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?
    {
        publish_committed_mutation(state, settled, RepoChangeReason::InviteUpdated).await;
    }
    Ok(())
}

fn invite_email_message(
    email_id: &str,
    repo: &Repository,
    invite: &RepositoryInvite,
    inviter_email: &str,
    secret: &str,
    now: u64,
) -> Result<InviteEmailMessage, ApiError> {
    let origin = public_app_origin("building repository invite email")?;
    let link = format!("{}/invites/{secret}", origin.trim_end_matches('/'));
    let owner = &repo.record.owner_handle;
    let repository = format!("{owner}/{}", repo.record.name);
    let push = if invite.permissions.can_push {
        " You'll also be able to push changes."
    } else {
        ""
    };
    let access =
        format!("You'll be able to read private files and take part in maintainer reviews.{push}");
    // A resend keeps the original expiry, so say how long is actually left.
    let expiry = match invite.expires_at_unix.saturating_sub(now) / (24 * 60 * 60) {
        0 => "in less than a day".to_string(),
        1 => "in 1 day".to_string(),
        days => format!("in {days} days"),
    };
    let footer = format!(
        "This invitation is for {}. Sign in or create an account with that email. \
         It expires {expiry}. If you weren't expecting it, you can ignore it. \
         Nothing changes until you accept.",
        invite.invited_email
    );
    Ok(InviteEmailMessage {
        idempotency_key: email_id.to_string(),
        to: invite.invited_email.clone(),
        reply_to: inviter_email.to_string(),
        subject: format!("@{owner} invited you to {repository}"),
        text: format!(
            "@{owner} invited you to become a member of {repository} on Scope.\n\n\
             {access}\n\nView invitation: {link}\n\n{footer}\n"
        ),
        html: format!(
            "<p>@{owner} invited you to become a member of <strong>{repository}</strong> on Scope.</p>\
             <p>{access}</p>\
             <p><a href=\"{link}\">View invitation</a></p>\
             <p style=\"color:#666;font-size:13px\">{footer}</p>",
            owner = escape_html(owner),
            repository = escape_html(&repository),
            access = escape_html(&access),
            footer = escape_html(&footer),
        ),
    })
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl AppState {
    pub(crate) fn start_invite_email_delivery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    deliver_due_invite_emails(&state, unix_now()?).await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "invite email delivery pass failed; retrying"
                    );
                }
                tokio::select! {
                    _ = state.invite_email_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        account::UserAccount, policy::Visibility, repo_collaboration::REPOSITORY_INVITE_TTL_SECS,
    };

    #[test]
    fn a_resent_email_states_the_time_left_and_escapes_what_the_owner_typed() {
        let owner = UserAccount {
            id: "user_owner".into(),
            handle: "owner".into(),
            email: "owner@example.com".into(),
            email_verified: true,
        };
        let repo = Repository::new(&owner, "repo", Visibility::Private, "repoi_test").unwrap();
        let invite = RepositoryInvite {
            id: "invite".into(),
            repo_id: repo.record.id.clone(),
            invited_email: "<b>odd</b>@example.com".into(),
            invited_email_normalized: "<b>odd</b>@example.com".into(),
            permissions: Default::default(),
            invited_by_user_id: owner.id.clone(),
            link_hashes: Vec::new(),
            created_at_unix: 0,
            updated_at_unix: 0,
            expires_at_unix: REPOSITORY_INVITE_TTL_SECS,
            accepted_by_user_id: None,
            accepted_at_unix: None,
            revoked_at_unix: None,
        };
        let day = 24 * 60 * 60;

        let fresh =
            invite_email_message("email", &repo, &invite, &owner.email, "secret", 0).unwrap();
        let late =
            invite_email_message("email", &repo, &invite, &owner.email, "secret", 6 * day + 1)
                .unwrap();

        assert!(fresh.text.contains("It expires in 7 days."));
        assert!(late.text.contains("It expires in less than a day."));
        assert!(fresh.text.contains("/invites/secret\n"));
        assert_eq!(fresh.reply_to, "owner@example.com");
        assert_eq!(fresh.idempotency_key, "email");
        assert!(fresh.html.contains("&lt;b&gt;odd&lt;/b&gt;@example.com"));
        assert!(!fresh.html.contains("<b>odd"));
    }
}
