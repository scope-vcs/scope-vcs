use super::*;
use crate::error::DomainErrorKind;
use crate::repository::access::RepositoryActor;
use crate::views::Views;

fn maintainer() -> RepositoryAccess {
    RepositoryAccess {
        actor: RepositoryActor::Owner,
        view: Views::builtin().full().clone(),
        can_push: true,
        can_change_file_visibility: true,
        can_manage_members: true,
        can_delete_repo: true,
    }
}

fn inputs(logins: &[&str]) -> Vec<GitHubReviewBotInput> {
    logins
        .iter()
        .map(|login| GitHubReviewBotInput {
            login: login.to_string(),
        })
        .collect()
}

fn invalid(logins: &[&str]) -> String {
    let error =
        set_github_review_bots(maintainer(), Some(&connection()), inputs(logins)).unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::InvalidInput, "{logins:?}");
    error.message
}

fn connection() -> GitHubConnection {
    GitHubConnection {
        repository_id: "owner/repo".to_string(),
        installation_id: 1,
        github_repository_id: 2,
        github_full_name: "octo/repo".to_string(),
        connected_by: None,
        connected_at_unix: 1,
        status: crate::github_connection::GitHubConnectionStatus::Connected,
        visibility: crate::github_connection::GitHubRepositoryVisibility::Private,
    }
}

fn account(login: &str, account_type: GitHubAccountType) -> GitHubAccount {
    GitHubAccount::new(7, login, account_type, None).unwrap()
}

#[test]
fn review_bots_are_github_logins_listed_once_by_maintainers() {
    let bots = set_github_review_bots(
        maintainer(),
        Some(&connection()),
        inputs(&[
            " greptile-apps[bot] ",
            "chatgpt-codex-connector[bot]",
            "octo-cat",
        ]),
    )
    .unwrap();
    assert_eq!(
        bots.iter()
            .map(|bot| bot.login.as_str())
            .collect::<Vec<_>>(),
        [
            "greptile-apps[bot]",
            "chatgpt-codex-connector[bot]",
            "octo-cat"
        ]
    );
    assert!(
        set_github_review_bots(maintainer(), Some(&connection()), Vec::new())
            .unwrap()
            .is_empty()
    );

    for login in [
        "",
        "  ",
        "[bot]",
        "-octo",
        "octo-",
        "octo cat",
        "octo/cat",
        "octo[bot]x",
    ] {
        invalid(&[login]);
    }
    invalid(&[&"a".repeat(40)]);
    assert_eq!(
        invalid(&["greptile-apps[bot]", "Greptile-Apps[bot]"]),
        "Greptile-Apps[bot] is listed twice."
    );
    let many = (0..=GITHUB_REVIEW_BOTS_LIMIT)
        .map(|index| format!("bot-{index}"))
        .collect::<Vec<_>>();
    invalid(&many.iter().map(String::as_str).collect::<Vec<_>>());

    assert_eq!(
        set_github_review_bots(maintainer(), None, inputs(&["greptile-apps[bot]"]))
            .unwrap_err()
            .kind,
        DomainErrorKind::NotFound
    );
    assert_eq!(
        set_github_review_bots(
            RepositoryAccess::public(),
            Some(&connection()),
            inputs(&["greptile-apps[bot]"])
        )
        .unwrap_err()
        .kind,
        DomainErrorKind::Forbidden
    );
}

#[test]
fn listed_bots_and_users_who_can_write_are_imported() {
    let bots = set_github_review_bots(
        maintainer(),
        Some(&connection()),
        inputs(&["greptile-apps[bot]"]),
    )
    .unwrap();
    let listed = github_author_standing(
        &bots,
        &account("GREPTILE-apps[bot]", GitHubAccountType::Bot),
    );
    assert_eq!(listed, GitHubAuthorStanding::ListedReviewBot);
    assert!(may_import_github_author(&listed, None));

    let unlisted_bot = github_author_standing(&bots, &account("spam[bot]", GitHubAccountType::Bot));
    assert_eq!(unlisted_bot, GitHubAuthorStanding::Unlisted);
    assert!(!may_import_github_author(
        &unlisted_bot,
        Some(GitHubRepositoryPermission::Admin)
    ));

    let user = github_author_standing(&bots, &account("octocat", GitHubAccountType::User));
    assert_eq!(user, GitHubAuthorStanding::NeedsPermission);
    for (permission, imported) in [
        (Some(GitHubRepositoryPermission::Admin), true),
        (Some(GitHubRepositoryPermission::Write), true),
        (Some(GitHubRepositoryPermission::Read), false),
        (Some(GitHubRepositoryPermission::None), false),
        (None, false),
    ] {
        assert_eq!(
            may_import_github_author(&user, permission),
            imported,
            "{permission:?}"
        );
    }
}

#[test]
fn review_bots_belong_to_one_live_link() {
    let linked = connection();
    assert!(github_review_bots_survive(Some(&linked), &linked));
    assert!(!github_review_bots_survive(None, &linked));
    let other_repository = GitHubConnection {
        github_repository_id: 3,
        ..connection()
    };
    assert!(!github_review_bots_survive(
        Some(&linked),
        &other_repository
    ));
    let other_installation = GitHubConnection {
        installation_id: 9,
        ..connection()
    };
    assert!(!github_review_bots_survive(
        Some(&linked),
        &other_installation
    ));
    let disconnected = GitHubConnection {
        status: crate::github_connection::GitHubConnectionStatus::Disconnected {
            reason: crate::github_connection::GitHubDisconnectReason::AppUninstalled,
            at_unix: 2,
        },
        ..connection()
    };
    assert!(!github_review_bots_survive(Some(&disconnected), &linked));
}
