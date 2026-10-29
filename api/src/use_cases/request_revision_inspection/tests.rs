use super::*;
use crate::error::ErrorKind;
use scope_domain::policy::LabelRule;
use scope_domain::requests::FileLineChanges;
use scope_domain::views::ViewId;

fn inspect_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<InspectedRequestChanges, ApiError> {
    super::inspect_counted_request_changes(
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
        :100644 100755 old new M\0m.txt\0\
        0\t4\tz.txt\0-\t-\tnested//type.txt\0\
        12\t0\ta.txt\0\
        3\t1\tm.txt\0";
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
            .map(|file| file.file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.txt", "m.txt", "nested//type.txt", "z.txt"]
    );
    assert_eq!(files[0].file.kind, FileChangeKind::Added);
    assert_eq!(files[0].file.old_mode, None);
    assert_eq!(files[0].file.old_oid, None);
    assert_eq!(files[0].file.new_mode.as_deref(), Some("100755"));
    assert_eq!(files[0].file.new_oid.as_deref(), Some("new"));
    for file in &files[1..3] {
        assert_eq!(file.file.kind, FileChangeKind::Modified);
        assert_eq!(file.file.old_oid.as_deref(), Some("old"));
        assert_eq!(file.file.new_oid.as_deref(), Some("new"));
        assert_eq!(file.file.old_mode.as_deref(), Some("100644"));
    }
    assert_eq!(files[1].file.new_mode.as_deref(), Some("100755"));
    assert_eq!(files[2].file.new_mode.as_deref(), Some("120000"));
    assert_eq!(files[3].file.kind, FileChangeKind::Deleted);
    assert_eq!(files[3].file.old_mode.as_deref(), Some("100644"));
    assert_eq!(files[3].file.old_oid.as_deref(), Some("old"));
    assert_eq!(files[3].file.new_mode, None);
    assert_eq!(files[3].file.new_oid, None);
    assert!(files.iter().all(|file| file.file.label == ViewId::public()));
    assert_eq!(
        files.iter().map(|file| file.lines).collect::<Vec<_>>(),
        [
            FileLineChanges::Text {
                additions: 12,
                deletions: 0
            },
            FileLineChanges::Text {
                additions: 3,
                deletions: 1
            },
            FileLineChanges::Binary,
            FileLineChanges::Text {
                additions: 0,
                deletions: 4
            },
        ]
    );
}

#[test]
fn mixed_visibility_records_hidden_paths_without_hiding_readable_changes() {
    let mut policy = Policy::new(ViewId::private());
    policy
        .add_rule(LabelRule::public(ScopePath::parse("/public").unwrap()))
        .unwrap();
    let changes = b":100644 100644 old new M\0private.txt\0\
        :100644 100644 old new M\0public//file.txt\0\
        1\t1\tprivate.txt\0\
        2\t2\tpublic//file.txt\0";
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
            assert_eq!(result.files[0].file.label, ViewId::private());
        }
        let public = result.files.last().unwrap();
        assert_eq!(public.file.path, "public//file.txt");
        assert_eq!(public.file.label, ViewId::public());
    }
    let hidden = inspect_request_changes(
        b":100644 100644 old new M\0private.txt\0\
        :100644 100644 old new R100\0renamed.txt\0\
        1\t1\tprivate.txt\0\
        1\t1\trenamed.txt\0",
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
            &b":100644 100644 old new\0file\x001\t0\tfile\0"[..],
            ApiError::internal_message("invalid request diff header :100644 100644 old new"),
        ),
        (
            &b"100644 100644 old new M\0file\0"[..],
            ApiError::internal_message("invalid request diff line count 100644 100644 old new M"),
        ),
        (
            &b":100644 100644 old new M"[..],
            ApiError::internal_message("request diff is missing a path"),
        ),
        (
            &b":100644 100644 old new R100\0file\x001\t0\tfile\0"[..],
            ApiError::internal_message("unsupported request diff status R100"),
        ),
        (
            &b":100644 100644 old new M\0file\0"[..],
            ApiError::internal_message("request diff line counts do not match its changes"),
        ),
        (
            &b":100644 100644 old new M\0a\0:100644 100644 old new M\0b\x001\t0\tb\x001\t0\ta\0"[..],
            ApiError::internal_message("request diff line counts are out of order"),
        ),
        (
            &b":100644 100644 old new M\0file\0x\t0\tfile\0"[..],
            ApiError::internal_message("invalid request diff line count x"),
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
        ApiError::internal_message("invalid request diff line count malformed")
            .operator_diagnostic()
    );
    for bytes in [
        &b":\xff\0file\x001\t0\tfile\0"[..],
        &b":100644 100644 old new M\0\xff\x001\t0\t\xff\0"[..],
        &b":100644 100644 old new M\0dir/../file\x001\t0\tdir/../file\0"[..],
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
        b"\0\0:100644 100644 old new M100\0file\x000\t0\tfile",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].file.kind, FileChangeKind::Modified);
    assert_eq!(result.files[0].file.path, "file");
    let empty_path = inspect_request_changes(
        b":100644 100644 old new M\0\x000\t0\t\0",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert_eq!(empty_path.files[0].file.path, "");
    let empty = inspect_request_changes(
        b"\0",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert!(empty.files.is_empty());
    assert!(!empty.hidden);
}

#[test]
fn uncounted_changes_need_no_line_counts_and_still_hide_unreadable_paths() {
    let mut policy = Policy::new(ViewId::private());
    policy
        .add_rule(LabelRule::public(ScopePath::parse("/public").unwrap()))
        .unwrap();
    let changes = super::inspect_request_changes(
        b":100644 100644 old new M\0public/b.txt\0\
        :000000 100644 added new A\0public/a.txt\0\
        :100644 100644 old new M\0private.txt\0",
        &policy,
        &scope_domain::views::Views::builtin(),
        &RepositoryAccess::public(),
    )
    .unwrap();
    assert!(changes.hidden);
    assert_eq!(
        changes
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.kind))
            .collect::<Vec<_>>(),
        [
            ("public/a.txt", FileChangeKind::Added),
            ("public/b.txt", FileChangeKind::Modified)
        ]
    );
}
