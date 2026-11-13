use super::*;
use crate::error::ErrorKind;
use scope_domain::policy::LabelRule;
use scope_domain::requests::FileLineChanges;
use scope_domain::views::ViewId;

fn inspect_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<Option<Vec<InspectedRequestChange>>, ApiError> {
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
        :100644 100755 old new R087\0from.txt\0zz.txt\0\
        0\t4\tz.txt\0-\t-\tnested//type.txt\0\
        12\t0\ta.txt\0\
        3\t1\tm.txt\0\
        2\t1\t\0from.txt\0zz.txt\0";
    let mut files = inspect_request_changes(
        changes,
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap()
    .unwrap();
    let renamed = files.pop().unwrap();
    assert_eq!(renamed.file.path, "zz.txt");
    assert_eq!(renamed.file.old_path.as_deref(), Some("from.txt"));
    assert_eq!(renamed.file.kind, FileChangeKind::Renamed);
    assert_eq!(renamed.file.old_mode.as_deref(), Some("100644"));
    assert_eq!(renamed.file.new_mode.as_deref(), Some("100755"));
    assert_eq!(renamed.file.old_oid.as_deref(), Some("old"));
    assert_eq!(renamed.file.new_oid.as_deref(), Some("new"));
    assert_eq!(
        renamed.lines,
        FileLineChanges::Text {
            additions: 2,
            deletions: 1
        }
    );
    assert_eq!(
        files
            .iter()
            .map(|file| file.file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.txt", "m.txt", "nested//type.txt", "z.txt"]
    );
    assert!(files.iter().all(|file| file.file.old_path.is_none()));
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
fn counted_changes_with_any_unreadable_path_are_withheld() {
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
        let Some(files) = result else {
            assert!(!full_view);
            continue;
        };
        assert!(full_view);
        assert_eq!(
            files
                .iter()
                .map(|file| (file.file.path.as_str(), file.file.label.clone()))
                .collect::<Vec<_>>(),
            [
                ("private.txt", ViewId::private()),
                ("public//file.txt", ViewId::public())
            ]
        );
    }
    let partial_rename = inspect_request_changes(
        b":100644 100644 old new R100\0private.txt\0public/file.txt\0\
        0\t0\t\0private.txt\0public/file.txt\0",
        &policy,
        RepositoryAccess::public(),
    )
    .unwrap();
    assert!(partial_rename.is_none());
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
            &b":100644 100644 old new C100\0file\x001\t0\tfile\0"[..],
            ApiError::internal_message("unsupported request diff status C100"),
        ),
        (
            &b":100644 100644 old new R100\0from\0to\x001\t0\t\0from\0other\0"[..],
            ApiError::internal_message("request diff line counts are out of order"),
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
    .unwrap()
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file.kind, FileChangeKind::Modified);
    assert_eq!(result[0].file.path, "file");
    let empty = inspect_request_changes(
        b"\0",
        &Policy::new(ViewId::public()),
        RepositoryAccess::public(),
    )
    .unwrap();
    assert_eq!(empty.map(|files| files.len()), Some(0));
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
        :100644 100644 old new M\0private.txt\0\
        :100644 100644 old new R090\0private-source.txt\0public/moved.txt\0\
        :100644 100644 old new R090\0public/gone.txt\0private-target.txt\0\
        :100644 100644 old new R090\0public/from.txt\0public/to.txt\0",
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
            ("public/b.txt", FileChangeKind::Modified),
            ("public/gone.txt", FileChangeKind::Deleted),
            ("public/moved.txt", FileChangeKind::Added),
            ("public/to.txt", FileChangeKind::Renamed),
        ]
    );
    let gone = &changes.files[2];
    assert_eq!(
        (gone.new_mode.as_deref(), gone.new_oid.as_deref()),
        (None, None)
    );
    let moved = &changes.files[3];
    assert_eq!(
        (moved.old_mode.as_deref(), moved.old_oid.as_deref()),
        (None, None)
    );
    assert_eq!(
        changes.files[4].old_path.as_deref(),
        Some("public/from.txt")
    );
}
