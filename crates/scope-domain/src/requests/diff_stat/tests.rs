use super::*;

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn text(additions: u64, deletions: u64) -> FileLineChanges {
    FileLineChanges::Text {
        additions,
        deletions,
    }
}

fn file(value: &str, lines: FileLineChanges) -> FileDiffStat {
    FileDiffStat {
        path: path(value),
        lines,
        renamed_from: None,
    }
}

fn stat(additions: u64, deletions: u64, changed_files: u64) -> DiffStat {
    DiffStat {
        additions,
        deletions,
        changed_files,
    }
}

#[test]
fn readers_count_only_files_they_can_read_and_binary_files_add_no_lines() {
    let mut builder = RevisionDiffStatBuilder::new(3);
    builder.add(file("/public.rs", text(5, 2)));
    builder.add(file("/image.png", FileLineChanges::Binary));
    builder.add(file("/secret.rs", text(40, 1)));
    let recorded = builder.finish();

    assert_eq!(recorded.totals(), stat(45, 3, 3));
    assert_eq!(
        recorded.readable_by(|file| file.as_str() != "/secret.rs"),
        Some(stat(5, 2, 2))
    );
}

#[test]
fn renames_count_as_renames_only_for_readers_of_both_paths() {
    let mut builder = RevisionDiffStatBuilder::new(1);
    builder.add(FileDiffStat {
        renamed_from: Some(RenameSource {
            path: path("/old.rs"),
            old_file: text(0, 90),
            new_file: text(92, 0),
        }),
        ..file("/new.rs", text(3, 1))
    });
    let recorded = builder.finish();

    assert_eq!(recorded.totals(), stat(3, 1, 1));
    for (readable, expected) in [
        (&["/old.rs", "/new.rs"][..], Some(stat(3, 1, 1))),
        (&["/new.rs"], Some(stat(92, 0, 1))),
        (&["/old.rs"], Some(stat(0, 90, 1))),
        (&[], Some(stat(0, 0, 0))),
    ] {
        assert_eq!(
            recorded.readable_by(|file| readable.contains(&file.as_str())),
            expected,
            "{readable:?}"
        );
    }
}

#[test]
fn unlisted_files_keep_totals_but_leave_restricted_readers_without_counts() {
    let mut capped = RevisionDiffStatBuilder::new(1);
    capped.add(file("/a.rs", text(1, 0)));
    capped.add(file("/b.rs", text(2, 3)));
    let mut unnamed = RevisionDiffStatBuilder::new(10);
    unnamed.add_unlisted(text(1, 1));
    unnamed.add(file("/c.rs", text(1, 0)));

    for (recorded, totals) in [
        (capped.finish(), stat(3, 3, 2)),
        (unnamed.finish(), stat(2, 1, 2)),
    ] {
        assert_eq!(recorded.totals(), totals);
        assert_eq!(recorded.files(), None);
        assert_eq!(recorded.readable_by(|_| true), None);
    }
}
