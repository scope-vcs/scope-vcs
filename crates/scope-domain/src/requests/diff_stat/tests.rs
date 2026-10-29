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

#[test]
fn readers_count_only_files_they_can_read_and_binary_files_add_no_lines() {
    let mut builder = RevisionDiffStatBuilder::new(3);
    builder.add(Some(path("/public.rs")), text(5, 2));
    builder.add(Some(path("/image.png")), FileLineChanges::Binary);
    builder.add(Some(path("/secret.rs")), text(40, 1));
    let recorded = builder.finish();

    assert_eq!(
        recorded.totals(),
        DiffStat {
            additions: 45,
            deletions: 3,
            changed_files: 3,
        }
    );
    assert_eq!(
        recorded.readable_by(|file| file.as_str() != "/secret.rs"),
        Some(DiffStat {
            additions: 5,
            deletions: 2,
            changed_files: 2,
        })
    );
}

#[test]
fn unlisted_files_keep_totals_but_leave_restricted_readers_without_counts() {
    let mut capped = RevisionDiffStatBuilder::new(1);
    capped.add(Some(path("/a.rs")), text(1, 0));
    capped.add(Some(path("/b.rs")), text(2, 3));
    let mut unnamed = RevisionDiffStatBuilder::new(10);
    unnamed.add(None, text(1, 1));
    unnamed.add(Some(path("/c.rs")), text(1, 0));

    for (recorded, totals) in [
        (
            capped.finish(),
            DiffStat {
                additions: 3,
                deletions: 3,
                changed_files: 2,
            },
        ),
        (
            unnamed.finish(),
            DiffStat {
                additions: 2,
                deletions: 1,
                changed_files: 2,
            },
        ),
    ] {
        assert_eq!(recorded.totals(), totals);
        assert_eq!(recorded.files(), None);
        assert_eq!(recorded.readable_by(|_| true), None);
    }
}
