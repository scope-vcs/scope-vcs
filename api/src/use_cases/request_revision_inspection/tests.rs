use super::*;
use crate::error::ErrorKind;
use scope_domain::policy::VisibilityRule;

const ORDERS: [DiffStatusValidationOrder; 2] = [
    DiffStatusValidationOrder::BeforePath,
    DiffStatusValidationOrder::AfterVisibility,
];

#[test]
fn changes_preserve_kinds_modes_oids_raw_paths_and_sorting() {
    let changes = b":100644 000000 old deleted D\0z.txt\0\
        :100644 120000 old new T\0nested//type.txt\0\
        :000000 100755 added new A\0a.txt\0\
        :100644 100755 old new M\0m.txt\0";
    for order in ORDERS {
        let inspected = inspect_request_changes(
            changes,
            &Policy::new(Visibility::Public),
            RepositoryAccess::public(),
            order,
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
        assert_eq!(files[2].scope_path.as_str(), "/nested/type.txt");
        assert_eq!(files[3].kind, FileChangeKind::Deleted);
        assert_eq!(files[3].old_mode.as_deref(), Some("100644"));
        assert_eq!(files[3].old_oid.as_deref(), Some("old"));
        assert_eq!(files[3].new_mode, None);
        assert_eq!(files[3].new_oid, None);
        assert!(
            files
                .iter()
                .all(|file| file.visibility == Visibility::Public)
        );
    }
}

#[test]
fn mixed_visibility_records_hidden_paths_without_hiding_readable_changes() {
    let mut policy = Policy::new(Visibility::Private);
    policy
        .add_rule(VisibilityRule::public(ScopePath::parse("/public").unwrap()))
        .unwrap();
    let changes = b":100644 100644 old new M\0private.txt\0\
        :100644 100644 old new M\0public//file.txt\0";
    for order in ORDERS {
        for can_read_private_files in [false, true] {
            let result = inspect_request_changes(
                changes,
                &policy,
                RepositoryAccess {
                    can_read_private_files,
                    ..RepositoryAccess::public()
                },
                order,
            )
            .unwrap();
            assert_eq!(result.hidden, !can_read_private_files);
            assert_eq!(
                result.files.len(),
                if can_read_private_files { 2 } else { 1 }
            );
            if can_read_private_files {
                assert_eq!(result.files[0].visibility, Visibility::Private);
            }
            let public = result.files.last().unwrap();
            assert_eq!(public.path, "public//file.txt");
            assert_eq!(public.scope_path.as_str(), "/public/file.txt");
            assert_eq!(public.visibility, Visibility::Public);
        }
        let hidden = inspect_request_changes(
            changes,
            &Policy::new(Visibility::Private),
            RepositoryAccess::public(),
            order,
        )
        .unwrap();
        assert!(hidden.hidden);
        assert!(hidden.files.is_empty());
    }
}

#[test]
fn malformed_records_keep_their_error_categories_and_diagnostics() {
    for order in ORDERS {
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
                &Policy::new(Visibility::Public),
                RepositoryAccess::public(),
                order,
            )
            .unwrap_err();
            assert_eq!(error.kind, expected.kind);
            assert_eq!(error.public_message(), expected.public_message());
            assert_eq!(error.operator_diagnostic(), expected.operator_diagnostic());
        }
        for bytes in [
            &b"\xff\0file\0"[..],
            &b":100644 100644 old new M\0\xff\0"[..],
            &b":100644 100644 old new M\0dir/../file\0"[..],
        ] {
            let error = inspect_request_changes(
                bytes,
                &Policy::new(Visibility::Private),
                RepositoryAccess::public(),
                order,
            )
            .unwrap_err();
            assert_eq!(error.kind, ErrorKind::BadRequest);
        }
    }
}

#[test]
fn reader_keeps_existing_permissive_framing_and_status_interpretation() {
    for order in ORDERS {
        let result = inspect_request_changes(
            b"\0\0:100644 100644 old new M100\0file",
            &Policy::new(Visibility::Public),
            RepositoryAccess::public(),
            order,
        )
        .unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].kind, FileChangeKind::Modified);
        assert_eq!(result.files[0].path, "file");
        let empty_path = inspect_request_changes(
            b":100644 100644 old new M\0",
            &Policy::new(Visibility::Public),
            RepositoryAccess::public(),
            order,
        )
        .unwrap();
        assert_eq!(empty_path.files[0].path, "");
        assert_eq!(empty_path.files[0].scope_path, ScopePath::root());
        let empty = inspect_request_changes(
            b"\0",
            &Policy::new(Visibility::Public),
            RepositoryAccess::public(),
            order,
        )
        .unwrap();
        assert!(empty.files.is_empty());
        assert!(!empty.hidden);
    }
}
