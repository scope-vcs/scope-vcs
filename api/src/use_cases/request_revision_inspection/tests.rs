use super::*;
use crate::error::ErrorKind;
use scope_domain::policy::LabelRule;
use scope_domain::views::ViewId;

fn inspect_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<InspectedRequestChanges, ApiError> {
    super::inspect_request_changes(
        changes,
        policy,
        &scope_domain::views::Views::builtin(),
        &access,
    )
}

#[test]
fn changes_preserve_kinds_modes_oids_raw_paths_and_sorting() {
    let changes = b":100644 000000 old deleted D\0z.txt\0\
        :100644 120000 old new T\0nested//type.txt\0\
        :000000 100755 added new A\0a.txt\0\
        :100644 100755 old new M\0m.txt\0";
    let inspected = inspect_request_changes(
        changes,
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert!(!inspected.hidden);
    let files = inspected.files;
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.txt", "m.txt", "nested//type.txt", "z.txt"]
    );
    assert_eq!(files[0].kind, FileChangeKind::Added);
    assert_eq!(files[0].old_mode, None);
    assert_eq!(files[0].old_oid, None);
    assert_eq!(files[0].new_mode.as_deref(), Some("100755"));
    assert_eq!(files[0].new_oid.as_deref(), Some("new"));
    for file in &files[1..3] {
        assert_eq!(file.kind, FileChangeKind::Modified);
        assert_eq!(file.old_oid.as_deref(), Some("old"));
        assert_eq!(file.new_oid.as_deref(), Some("new"));
        assert_eq!(file.old_mode.as_deref(), Some("100644"));
    }
    assert_eq!(files[1].new_mode.as_deref(), Some("100755"));
    assert_eq!(files[2].new_mode.as_deref(), Some("120000"));
    assert_eq!(files[3].kind, FileChangeKind::Deleted);
    assert_eq!(files[3].old_mode.as_deref(), Some("100644"));
    assert_eq!(files[3].old_oid.as_deref(), Some("old"));
    assert_eq!(files[3].new_mode, None);
    assert_eq!(files[3].new_oid, None);
    assert!(files.iter().all(|file| file.label == ViewId::public()));
}

#[test]
fn mixed_visibility_records_hidden_paths_without_hiding_readable_changes() {
    let mut policy = Policy::new(ViewId::private());
    policy
        .add_rule(LabelRule::public(ScopePath::parse("/public").unwrap()))
        .unwrap();
    let changes = b":100644 100644 old new M\0private.txt\0\
        :100644 100644 old new M\0public//file.txt\0";
    for full_view in [false, true] {
        let result = inspect_request_changes(
            changes,
            &policy,
            RepositoryAccess {
                view: if full_view {
                    ViewId::private()
                } else {
                    ViewId::public()
                },
                ..RepositoryAccess::public()
            },
        )
        .unwrap();
        assert_eq!(result.hidden, !full_view);
        assert_eq!(result.files.len(), if full_view { 2 } else { 1 });
        if full_view {
            assert_eq!(result.files[0].label, ViewId::private());
        }
        let public = result.files.last().unwrap();
        assert_eq!(public.path, "public//file.txt");
        assert_eq!(public.label, ViewId::public());
    }
    let hidden = inspect_request_changes(
        &[&changes[..], b":100644 100644 old new R100\0renamed.txt\0"].concat(),
        &Policy::new(ViewId::private()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert!(hidden.hidden);
    assert!(hidden.files.is_empty());
}

#[test]
fn malformed_records_keep_their_error_categories_and_diagnostics() {
    for (bytes, expected) in [
        (
            &b":100644 100644 old new\0file\0"[..],
            ApiError::internal_message("invalid request diff header :100644 100644 old new"),
        ),
        (
            &b"100644 100644 old new M\0file\0"[..],
            ApiError::internal_message("invalid request diff header 100644 100644 old new M"),
        ),
        (
            &b":100644 100644 old new M"[..],
            ApiError::internal_message("request diff is missing a path"),
        ),
        (
            &b":100644 100644 old new R100\0file\0"[..],
            ApiError::internal_message("unsupported request diff status R100"),
        ),
    ] {
        let error = inspect_request_changes(
            bytes,
            &Policy::new(ViewId::public()),
            RepositoryAccess::public(),
        )
        .unwrap_err();
        assert_eq!(error.kind, expected.kind);
        assert_eq!(error.public_message(), expected.public_message());
        assert_eq!(error.operator_diagnostic(), expected.operator_diagnostic());
    }
    let error = inspect_request_changes(
        b":100644 100644 old new A\0private.txt\0malformed\0",
        &Policy::new(ViewId::private()),
        RepositoryAccess::public(),
    )
    .unwrap_err();
    assert_eq!(
        error.operator_diagnostic(),
        ApiError::internal_message("invalid request diff header malformed").operator_diagnostic()
    );
    for bytes in [
        &b"\xff\0file\0"[..],
        &b":100644 100644 old new M\0\xff\0"[..],
        &b":100644 100644 old new M\0dir/../file\0"[..],
    ] {
        let error = inspect_request_changes(
            bytes,
            &Policy::new(ViewId::private()),
            RepositoryAccess::public(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::BadRequest);
    }
}

#[test]
fn reader_keeps_existing_permissive_framing_and_status_interpretation() {
    let result = inspect_request_changes(
        b"\0\0:100644 100644 old new M100\0file",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].kind, FileChangeKind::Modified);
    assert_eq!(result.files[0].path, "file");
    let empty_path = inspect_request_changes(
        b":100644 100644 old new M\0",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert_eq!(empty_path.files[0].path, "");
    let empty = inspect_request_changes(
        b"\0",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert!(empty.files.is_empty());
    assert!(!empty.hidden);
}
