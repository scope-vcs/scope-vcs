use crate::{error::DomainError, repo_actions::ensure_repo_member, repository::Repository};

pub const MAX_REPOSITORY_DESCRIPTION_CHARS: usize = 160;
pub const MAX_REPOSITORY_WEBSITE_URL_CHARS: usize = 2048;

/// Changes public project context. Every maintainer can edit it, regardless of push permissions.
pub fn update_repo_metadata(
    repo: &mut Repository,
    user_id: &str,
    description: Option<String>,
    website_url: Option<String>,
) -> Result<bool, DomainError> {
    ensure_repo_member(repo, user_id)?;
    let description = optional_text(description);
    let website_url = optional_text(website_url);
    if let Some(description) = &description
        && (description.chars().count() > MAX_REPOSITORY_DESCRIPTION_CHARS
            || description.chars().any(char::is_control)
            || description.contains(['\u{2028}', '\u{2029}']))
    {
        return Err(DomainError::invalid_input(
            "description must be a single line of at most 160 characters",
        ));
    }
    if let Some(website_url) = &website_url {
        let url = url::Url::parse(website_url).map_err(|_| invalid_website_url())?;
        if website_url.chars().count() > MAX_REPOSITORY_WEBSITE_URL_CHARS
            || website_url.chars().any(char::is_whitespace)
            || website_url.chars().any(char::is_control)
            || !website_url.contains("://")
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid_website_url());
        }
    }
    if repo.record.description == description && repo.record.website_url == website_url {
        return Ok(false);
    }
    repo.record.description = description;
    repo.record.website_url = website_url;
    repo.bump_change_version();
    Ok(true)
}

fn optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn invalid_website_url() -> DomainError {
    DomainError::invalid_input(
        "website_url must be an absolute HTTP or HTTPS URL without credentials, at most 2048 characters",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        account::UserAccount,
        policy::Visibility,
        repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    };

    fn repository() -> Repository {
        Repository::new(
            &UserAccount {
                id: "owner-id".into(),
                handle: "owner".into(),
                email: "owner@example.com".into(),
                email_verified: true,
            },
            "repo",
            Visibility::Public,
            "repoi_test",
        )
        .unwrap()
    }

    #[test]
    fn metadata_is_normalized_and_only_changes_bump_the_version() {
        let mut repo = repository();
        assert!(
            update_repo_metadata(
                &mut repo,
                "owner-id",
                Some("  A useful project  ".into()),
                Some(" https://example.com/docs ".into())
            )
            .unwrap()
        );
        assert_eq!(repo.record.description.as_deref(), Some("A useful project"));
        assert_eq!(
            repo.record.website_url.as_deref(),
            Some("https://example.com/docs")
        );
        assert_eq!(repo.record.change_version, 2);
        assert!(
            !update_repo_metadata(
                &mut repo,
                "owner-id",
                Some("A useful project".into()),
                Some("https://example.com/docs".into())
            )
            .unwrap()
        );
        assert_eq!(repo.record.change_version, 2);
        assert!(update_repo_metadata(&mut repo, "owner-id", Some(" \n ".into()), None).unwrap());
        assert_eq!(repo.record.description, None);
        assert_eq!(repo.record.website_url, None);
    }

    #[test]
    fn invalid_metadata_never_partially_changes_the_repository() {
        let mut repo = repository();
        for website in [
            "javascript:alert(1)",
            "/docs",
            "https:example.com",
            "https://",
            "https://user:pass@example.com",
            "https://exam ple.com",
            "https://example.com/\npath",
        ] {
            assert!(
                update_repo_metadata(
                    &mut repo,
                    "owner-id",
                    Some("New description".into()),
                    Some(website.into())
                )
                .is_err(),
                "{website}"
            );
            assert_eq!(repo.record.description, None);
            assert_eq!(repo.record.change_version, 1);
        }
        for description in [
            "a".repeat(161),
            "two\nlines".into(),
            "two\u{2028}lines".into(),
        ] {
            assert!(update_repo_metadata(&mut repo, "owner-id", Some(description), None).is_err());
        }
        assert!(update_repo_metadata(&mut repo, "owner-id", Some("é".repeat(160)), None).unwrap());
    }

    #[test]
    fn every_member_can_edit_but_an_outsider_cannot() {
        let mut repo = repository();
        repo.members.push(RepositoryMember {
            user_id: "member-id".into(),
            repo_id: repo.record.id.clone(),
            permissions: RepositoryMemberPermissions::default(),
            created_at_unix: 1,
            updated_at_unix: 1,
        });
        assert!(
            update_repo_metadata(&mut repo, "member-id", Some("By a member".into()), None).unwrap()
        );
        assert!(update_repo_metadata(&mut repo, "outsider-id", None, None).is_err());
        assert_eq!(repo.record.description.as_deref(), Some("By a member"));
    }
}
