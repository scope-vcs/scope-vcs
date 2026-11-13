use super::*;
use crate::requests::{CheckCommitBase, RequestCheckEvaluationState, fixtures::open_request};
use crate::views::{ViewId, Views};

fn context(request: &Request) -> RequestCheckContext {
    RequestCheckContext::new(
        &request.id,
        "revision_1",
        &request.head_oid,
        request.base_version,
        request.base.review_base_oid(),
    )
    .unwrap()
}

fn reviewer() -> RequestCheckReviewer {
    RequestCheckReviewer::trusted()
}

fn request() -> Request {
    let mut request = open_request();
    request.main_ancestor_oid = "f".repeat(40);
    request.base = crate::requests::RequestBase::Main {
        review_base_oid: "f".repeat(40),
    };
    request.head_oid = "a".repeat(40);
    request
}

fn private_request() -> Request {
    Request {
        view: ViewId::private(),
        ..request()
    }
}

fn check_commit() -> GitHubTestedCommit {
    GitHubTestedCommit::CheckCommit {
        oid: "c".repeat(40),
        base: CheckCommitBase::new("d".repeat(40), "e".repeat(40)).unwrap(),
    }
}

#[test]
fn approval_rejects_an_evaluation_from_another_request_or_head_or_an_earlier_time() {
    let request = private_request();
    let required = ["ci / test".to_string()];
    let waiting = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap()
    .evaluation;
    for (request_id, head_oid) in [
        ("other_request".to_string(), request.head_oid.clone()),
        (request.id.clone(), "d".repeat(40)),
    ] {
        let evaluation = RequestCheckEvaluation {
            request_id,
            head_oid,
            ..waiting.clone()
        };
        assert_eq!(
            RequestCheckPlan::approve(&request, evaluation, reviewer(), 40)
                .unwrap_err()
                .message,
            "request check evaluation does not match the request head"
        );
    }
    assert_eq!(
        RequestCheckPlan::approve(&request, waiting, reviewer(), 29)
            .unwrap_err()
            .message,
        "request check approval cannot predate the evaluation"
    );
}

#[test]
fn github_checks_are_the_required_names_and_only_maintainer_heads_go_to_github_at_once() {
    let request = private_request();
    let required = ["ci / test".to_string(), "ci / lint".to_string()];

    let maintainer = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &required,
        },
        Some(reviewer()),
        30,
    )
    .unwrap();
    assert_eq!(
        maintainer.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert_eq!(
        maintainer.evaluation.checks,
        [
            RequestCheck {
                name: "ci / test".into()
            },
            RequestCheck {
                name: "ci / lint".into()
            },
        ]
    );
    assert_eq!(maintainer.evaluation.tested_oid, request.head_oid);
    assert!(maintainer.push_to_github);

    let contributor = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap();
    assert_eq!(
        contributor.evaluation.state,
        RequestCheckEvaluationState::AwaitingApproval
    );
    assert!(!contributor.push_to_github);

    let approved =
        RequestCheckPlan::approve(&request, contributor.evaluation.clone(), reviewer(), 40)
            .unwrap();
    assert_eq!(
        approved.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert!(approved.push_to_github);
    let closed = Request {
        closed_at_unix: Some(35),
        ..request
    };
    let approved_closed =
        RequestCheckPlan::approve(&closed, contributor.evaluation, reviewer(), 40).unwrap();
    assert!(!approved_closed.push_to_github);
}

#[test]
fn with_no_required_checks_only_a_maintainers_head_still_goes_to_github() {
    let request = private_request();
    let maintainer = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &[],
        },
        Some(reviewer()),
        30,
    )
    .unwrap();
    assert_eq!(
        maintainer.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    assert!(maintainer.push_to_github);
    let contributor = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &[],
        },
        None,
        30,
    )
    .unwrap();
    assert_eq!(
        contributor.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    assert!(!contributor.push_to_github);
}

#[test]
fn a_public_contribution_tests_its_check_commit_and_waits_for_approval_like_any_head() {
    let request = request();
    let required = ["ci / test".to_string()];
    let contributor = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap();
    let evaluation = &contributor.evaluation;
    assert_eq!(evaluation.head_oid, request.head_oid);
    assert_eq!(evaluation.tested_oid, "c".repeat(40));
    assert_eq!(
        evaluation.check_commit_base,
        Some(CheckCommitBase::new("d".repeat(40), "e".repeat(40)).unwrap())
    );
    assert_eq!(
        evaluation.state,
        RequestCheckEvaluationState::AwaitingApproval
    );
    assert!(!contributor.push_to_github);

    let approved =
        RequestCheckPlan::approve(&request, contributor.evaluation, reviewer(), 40).unwrap();
    assert_eq!(approved.evaluation.tested_oid, "c".repeat(40));
    assert!(approved.push_to_github);

    let maintainer = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        Some(reviewer()),
        30,
    )
    .unwrap();
    assert_eq!(maintainer.evaluation.tested_oid, "c".repeat(40));
    assert!(maintainer.push_to_github);
}

#[test]
fn a_contribution_that_conflicts_with_private_code_is_a_configuration_error_and_sends_nothing() {
    let request = request();
    for maintainer in [None, Some(reviewer())] {
        let plan = RequestCheckPlan::evaluate_github(
            &request,
            &context(&request),
            &Views::builtin(),
            GitHubRequestChecks {
                github_repository_id: 123,
                tested: GitHubTestedCommit::Conflict,
                required_check_names: &["ci / test".to_string()],
            },
            maintainer,
            30,
        )
        .unwrap();
        assert_eq!(
            plan.evaluation.state,
            RequestCheckEvaluationState::ConfigurationError
        );
        assert_eq!(
            plan.evaluation.message.as_deref(),
            Some(PRIVATE_CODE_CONFLICT_MESSAGE)
        );
        assert_eq!(plan.evaluation.tested_oid, request.head_oid);
        assert_eq!(plan.evaluation.check_commit_base, None);
        assert!(!plan.push_to_github);
    }
}

#[test]
fn only_full_view_requests_test_their_head_on_github() {
    let views = crate::requests::fixtures::views_with_agent();
    let mut agent_request = request();
    agent_request.view = crate::requests::fixtures::agent();
    for (request, target) in [
        (request(), GitHubCheckTarget::CheckCommit),
        (agent_request.clone(), GitHubCheckTarget::CheckCommit),
        (private_request(), GitHubCheckTarget::Head),
    ] {
        assert_eq!(GitHubCheckTarget::for_request(&request, &views), target);
    }
    let required = ["ci / test".to_string()];
    let plan = RequestCheckPlan::evaluate_github(
        &agent_request,
        &context(&agent_request),
        &views,
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap();
    assert!(plan.evaluation.tests_check_commit());
    assert!(
        RequestCheckPlan::evaluate_github(
            &agent_request,
            &context(&agent_request),
            &views,
            GitHubRequestChecks {
                github_repository_id: 123,
                tested: GitHubTestedCommit::Head,
                required_check_names: &required
            },
            None,
            30
        )
        .is_err()
    );
}

#[test]
fn the_tested_commit_must_fit_the_requests_view() {
    for (request, tested) in [
        (request(), GitHubTestedCommit::Head),
        (private_request(), check_commit()),
        (private_request(), GitHubTestedCommit::Conflict),
    ] {
        assert_eq!(
            RequestCheckPlan::evaluate_github(
                &request,
                &context(&request),
                &Views::builtin(),
                GitHubRequestChecks {
                    github_repository_id: 123,
                    tested,
                    required_check_names: &[]
                },
                Some(reviewer()),
                30
            )
            .unwrap_err()
            .message,
            "the tested commit does not fit the request's view"
        );
    }
    let request = request();
    let head = GitHubTestedCommit::CheckCommit {
        oid: request.head_oid.clone(),
        base: CheckCommitBase::new("d".repeat(40), "e".repeat(40)).unwrap(),
    };
    assert!(
        RequestCheckPlan::evaluate_github(
            &request,
            &context(&request),
            &Views::builtin(),
            GitHubRequestChecks {
                github_repository_id: 123,
                tested: head,
                required_check_names: &[]
            },
            Some(reviewer()),
            30
        )
        .is_err()
    );
}

#[test]
fn a_check_commit_carries_private_code_and_a_public_head_does_not() {
    let required = ["ci / test".to_string()];
    let request = request();
    let check_commit = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap()
    .evaluation;
    assert!(check_commit.tests_check_commit());
    assert_eq!(
        check_commit.tested_code_view(ViewId::public()),
        ViewId::private()
    );
    let public_head = RequestCheckPlan::without_github(&request, &context(&request), 30)
        .unwrap()
        .evaluation;
    assert_eq!(
        public_head.tested_code_view(ViewId::public()),
        ViewId::public()
    );
    let private_head = RequestCheckPlan::evaluate_github(
        &private_request(),
        &context(&private_request()),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: GitHubTestedCommit::Head,
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap()
    .evaluation;
    assert_eq!(
        private_head.tested_code_view(ViewId::private()),
        ViewId::private()
    );
}

#[test]
fn started_checks_move_to_a_check_commit_on_new_private_main_and_send_it_at_once() {
    let request = request();
    let required = ["ci / test".to_string()];
    let started = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        Some(reviewer()),
        30,
    )
    .unwrap()
    .evaluation;
    let new_base = CheckCommitBase::new("1".repeat(40), "e".repeat(40)).unwrap();
    let renewed = RequestCheckPlan::rebuild_check_commit(
        &request,
        started.clone(),
        GitHubTestedCommit::CheckCommit {
            oid: "2".repeat(40),
            base: new_base.clone(),
        },
        40,
    )
    .unwrap();
    assert_eq!(renewed.evaluation.tested_oid, "2".repeat(40));
    assert_eq!(renewed.evaluation.check_commit_base, Some(new_base));
    assert_eq!(
        renewed.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert_eq!(renewed.evaluation.created_at_unix, 40);
    assert_eq!(renewed.evaluation.updated_at_unix, 40);
    assert!(renewed.push_to_github);
    assert_ne!(renewed.evaluation.id, started.id);
    assert_eq!(renewed.evaluation.checks, started.checks);
    assert_eq!(
        renewed.evaluation.github_repository_id,
        started.github_repository_id
    );

    let conflict = RequestCheckPlan::rebuild_check_commit(
        &request,
        started.clone(),
        GitHubTestedCommit::Conflict,
        40,
    )
    .unwrap();
    assert_eq!(
        conflict.evaluation.state,
        RequestCheckEvaluationState::ConfigurationError
    );
    assert_eq!(
        conflict.evaluation.message.as_deref(),
        Some(PRIVATE_CODE_CONFLICT_MESSAGE)
    );
    assert_eq!(conflict.evaluation.tested_oid, request.head_oid);
    assert_eq!(conflict.evaluation.created_at_unix, 40);
    assert!(!conflict.push_to_github);

    let waiting = RequestCheckPlan::evaluate_github(
        &request,
        &context(&request),
        &Views::builtin(),
        GitHubRequestChecks {
            github_repository_id: 123,
            tested: check_commit(),
            required_check_names: &required,
        },
        None,
        30,
    )
    .unwrap()
    .evaluation;
    let moved_on = Request {
        head_oid: "9".repeat(40),
        ..request.clone()
    };
    for (request, evaluation) in [(&request, waiting), (&moved_on, started)] {
        assert!(
            RequestCheckPlan::rebuild_check_commit(request, evaluation, check_commit(), 40)
                .is_err()
        );
    }
}
