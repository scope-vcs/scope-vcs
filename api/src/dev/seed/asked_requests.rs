use super::git_segments::SeedPoolPack;
use crate::error::ApiError;
use scope_domain::{
    account::UserAccount,
    github::account::{GitHubAccount, GitHubAccountType},
    repository::access::RepositoryAccess,
    requests::{
        AttachRequestChangeFacts, AttachRequestChangeInput, CloseRequestInput,
        CloseRequestMutation, EditRequestIdentityInput, OpenRequestFromAskFacts,
        OpenRequestFromAskInput, RecordWorkingRequestUploadInput, Request, RequestActor,
        RequestAsk, RequestCloseReason, RequestEvent, RequestWorkInput, attach_request_change,
        close_request, edit_request_identity, open_request_from_ask, record_working_request_upload,
        start_request_work,
    },
    views::{ViewId, Views},
};
use scope_postgres::db::CatalogFixture;

pub(super) struct SeedAskedRequests {
    pub(super) public_main_oid: String,
    pub(super) draft_head_oid: String,
    pub(super) draft_pack: SeedPoolPack,
}

struct SeedAsk {
    id: &'static str,
    asker: (u64, &'static str),
    github_issue_number: u64,
    title: &'static str,
    text_markdown: &'static str,
    asked_at_unix: u64,
    now_unix: u64,
}

const TOMASZ: (u64, &str) = (9_000_142, "tomasz-k");
const INES: (u64, &str) = (9_000_151, "ines-r");
const LOGIN_TIMEOUT_TEXT: &str = concat!(
    "On a slow connection the sign-in page gives up before the server answers. ",
    "I get “Session check failed” and have to reload.\n\n",
    "1. Throttle the network to Slow 3G in the browser’s dev tools.\n",
    "2. Open `/login` and submit valid credentials.\n",
    "3. Wait about 30 seconds.\n\n",
    "I expected sign-in to finish, or the page to keep waiting. The error appears at ",
    "exactly 30 seconds, even though the server responds at around 40.\n\n",
    "Version 2.14.1, Firefox on Linux.\n\n",
    "The same happens on a tethered phone connection, and a second attempt right after ",
    "the failure usually works because the session is already half set up.\n\n",
    "Raising the timeout would be enough for me. One retry before the error would be ",
    "better, since the server does answer in the end."
);
const CSV_HEADER_TEXT: &str = concat!(
    "Exporting the retry log to CSV writes the rows but not the header row, so the ",
    "columns have no names when the file opens in a spreadsheet.\n\n",
    "1. Open the retry log.\n",
    "2. Choose Export, then CSV.\n\n",
    "The first line of the file should name the columns."
);

pub(super) fn seed(
    catalog: &mut CatalogFixture,
    owner: &UserAccount,
    repo_id: &str,
    facts: SeedAskedRequests,
) -> Result<(), ApiError> {
    for (id, login) in [TOMASZ, INES] {
        catalog.github_accounts.push(GitHubAccount::new(
            id,
            login,
            GitHubAccountType::User,
            None,
        )?);
    }
    facts.draft_pack.add_to(catalog);
    let views = Views::builtin();
    let access = RepositoryAccess::full_view_reader(&views);
    let actor = RequestActor {
        access: &access,
        is_invitee: false,
        views: &views,
    };

    opened(
        catalog,
        repo_id,
        &facts.public_main_oid,
        SeedAsk {
            id: "req_demo_asked",
            asker: TOMASZ,
            github_issue_number: 142,
            title: "Login times out after 30 seconds on slow connections",
            text_markdown: LOGIN_TIMEOUT_TEXT,
            asked_at_unix: 1_782_000_000,
            now_unix: 1_790_000_600,
        },
    )?;

    let taken = opened(
        catalog,
        repo_id,
        &facts.public_main_oid,
        SeedAsk {
            id: "req_demo_asked_taken",
            asker: TOMASZ,
            github_issue_number: 97,
            title: "Rework sign-in for slow and offline networks",
            text_markdown: "Sign-in should queue while the network is down and finish when it returns.",
            asked_at_unix: 1_777_000_000,
            now_unix: 1_790_000_610,
        },
    )?;
    let taken = start_request_work(
        taken,
        actor,
        false,
        RequestWorkInput {
            request_id: "req_demo_asked_taken".to_string(),
            actor_user_id: owner.id.clone(),
            event_id: "event_req_demo_asked_taken_work_started".to_string(),
            now_unix: 1_790_000_611,
        },
    )?;
    saved(catalog, taken.request, [taken.event]);

    let draft = opened(
        catalog,
        repo_id,
        &facts.public_main_oid,
        SeedAsk {
            id: "req_demo_asked_draft",
            asker: INES,
            github_issue_number: 151,
            title: "Export to CSV drops the header row",
            text_markdown: CSV_HEADER_TEXT,
            asked_at_unix: 1_785_000_000,
            now_unix: 1_790_000_620,
        },
    )?;
    let attached = attach_request_change(
        draft,
        actor,
        AttachRequestChangeFacts::default(),
        AttachRequestChangeInput {
            request_id: "req_demo_asked_draft".to_string(),
            actor_user_id: owner.id.clone(),
            name: "csv-header-row".to_string(),
            main_oid: facts.public_main_oid.clone(),
            event_id: "event_req_demo_asked_draft_change_attached".to_string(),
            now_unix: 1_790_000_621,
        },
    )?;
    let uploaded = record_working_request_upload(
        attached.request,
        actor,
        RecordWorkingRequestUploadInput {
            request_id: "req_demo_asked_draft".to_string(),
            actor_user_id: owner.id.clone(),
            expected_old_head_oid: None,
            new_head_oid: facts.draft_head_oid,
            now_unix: 1_790_000_622,
        },
    )?;
    let described = edit_request_identity(
        uploaded.request,
        actor,
        false,
        EditRequestIdentityInput {
            request_id: "req_demo_asked_draft".to_string(),
            actor_user_id: owner.id.clone(),
            event_id: "event_req_demo_asked_draft_identity_edited".to_string(),
            title: None,
            description_markdown: Some(
                "Writes the column names as the first line of every CSV export.".to_string(),
            ),
            expected_description_markdown: None,
            now_unix: 1_790_000_623,
        },
    )?;
    saved(
        catalog,
        described.request,
        [attached.event, described.event],
    );

    let declined = opened(
        catalog,
        repo_id,
        &facts.public_main_oid,
        SeedAsk {
            id: "req_demo_asked_closed",
            asker: INES,
            github_issue_number: 104,
            title: "Billing page ignores the system theme",
            text_markdown: "The billing page stays light when the system theme is dark.",
            asked_at_unix: 1_779_000_000,
            now_unix: 1_790_000_630,
        },
    )?;
    let CloseRequestMutation::Closed { request, event } = close_request(
        declined,
        Vec::new(),
        Vec::new(),
        CloseRequestInput {
            request_id: "req_demo_asked_closed".to_string(),
            actor_user_id: owner.id.clone(),
            actor_is_maintainer: true,
            reason: Some(RequestCloseReason::NotPlanned),
            event_id: "event_req_demo_asked_closed_closed".to_string(),
            now_unix: 1_790_000_631,
        },
    )?
    else {
        return Err(ApiError::internal_message(
            "a seeded request with an ask was deleted instead of closed",
        ));
    };
    saved(catalog, request, [event]);
    Ok(())
}

fn opened(
    catalog: &mut CatalogFixture,
    repo_id: &str,
    main_oid: &str,
    ask: SeedAsk,
) -> Result<Request, ApiError> {
    let mutation = open_request_from_ask(
        OpenRequestFromAskFacts::default(),
        OpenRequestFromAskInput {
            id: ask.id.to_string(),
            repo_id: repo_id.to_string(),
            title: ask.title.to_string(),
            ask: RequestAsk {
                github_account_id: ask.asker.0,
                github_issue_number: ask.github_issue_number,
                source_url: format!(
                    "https://github.com/{repo_id}/issues/{}",
                    ask.github_issue_number
                ),
                text_markdown: ask.text_markdown.to_string(),
                asked_at_unix: ask.asked_at_unix,
            },
            view: ViewId::public(),
            main_oid: main_oid.to_string(),
            event_id: format!("event_{}_started", ask.id),
            now_unix: ask.now_unix,
        },
        &Views::builtin(),
    )?;
    saved(catalog, mutation.request.clone(), [mutation.event]);
    Ok(mutation.request)
}

fn saved(
    catalog: &mut CatalogFixture,
    request: Request,
    events: impl IntoIterator<Item = RequestEvent>,
) {
    catalog.requests.insert(request.id.clone(), request);
    catalog
        .request_events
        .extend(events.into_iter().map(|event| (event.id.clone(), event)));
}
