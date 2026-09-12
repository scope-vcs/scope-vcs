use super::parse_commit_paths;
use axum::http::StatusCode;
use scope_domain::{
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    repository::access::RepositoryAccess,
};

const ZERO_OID: &str = "0000000000000000000000000000000000000000";
const ONE_OID: &str = "1111111111111111111111111111111111111111";

#[test]
fn anchor_parser_preserves_status_before_path_validation() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/private.txt").unwrap(),
        ))
        .unwrap();

    for changes in [
        diff("R100", "private.txt"),
        header("R100"),
        diff("R100", "../private.txt"),
    ] {
        assert_api_error(
            parse_commit_paths(changes.as_bytes(), &policy, RepositoryAccess::public())
                .unwrap_err(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Scope hit an internal error.",
            "unsupported request diff status R100",
        );
    }

    assert_api_error(
        parse_commit_paths(header("A").as_bytes(), &policy, RepositoryAccess::public())
            .unwrap_err(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "Scope hit an internal error.",
        "request diff is missing a path",
    );
    assert_api_error(
        parse_commit_paths(
            diff("A", "../private.txt").as_bytes(),
            &policy,
            RepositoryAccess::public(),
        )
        .unwrap_err(),
        StatusCode::BAD_REQUEST,
        "path cannot contain empty segments, . or ..",
        "path cannot contain empty segments, . or ..",
    );
}

#[test]
fn anchor_parser_validates_records_after_a_hidden_change() {
    let policy = Policy::new(Visibility::Private);
    let changes = format!("{}\0private.txt\0malformed\0", header("A"));

    assert_api_error(
        parse_commit_paths(changes.as_bytes(), &policy, RepositoryAccess::public()).unwrap_err(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "Scope hit an internal error.",
        "invalid request diff header malformed",
    );
}

#[test]
fn anchor_parser_accepts_all_statuses_and_canonicalizes_paths() {
    let mut policy = Policy::new(Visibility::Public);
    policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/private.txt").unwrap(),
        ))
        .unwrap();
    let changes = [
        diff("A", "z//added.txt"),
        diff("M", "m-modified.txt"),
        diff("T", "a-type.txt"),
        diff("D", "d-deleted.txt"),
        diff("A", "private.txt"),
    ]
    .concat();

    let (paths, hidden) =
        parse_commit_paths(changes.as_bytes(), &policy, RepositoryAccess::public()).unwrap();

    assert!(hidden);
    assert_eq!(
        paths.iter().map(ScopePath::as_str).collect::<Vec<_>>(),
        vec![
            "/a-type.txt",
            "/d-deleted.txt",
            "/m-modified.txt",
            "/z/added.txt",
        ]
    );
}

fn header(status: &str) -> String {
    let (old_mode, new_mode) = match status.as_bytes().first() {
        Some(b'A') => ("000000", "100644"),
        Some(b'T') => ("100644", "100755"),
        Some(b'D') => ("100644", "000000"),
        _ => ("100644", "100644"),
    };
    format!(":{old_mode} {new_mode} {ZERO_OID} {ONE_OID} {status}")
}

fn diff(status: &str, path: &str) -> String {
    format!("{}\0{path}\0", header(status))
}

fn assert_api_error(
    error: crate::error::ApiError,
    status: StatusCode,
    public_message: &str,
    diagnostic: &str,
) {
    assert_eq!(error.status(), status);
    assert_eq!(error.public_message(), public_message);
    assert_eq!(error.operator_diagnostic(), diagnostic);
}
