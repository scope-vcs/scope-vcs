use super::*;
use crate::repository::git::fixtures::{main_pack, segment};

fn sized(mut pack: GitPoolPack, bytes: u64) -> GitPoolPack {
    if let GitPoolPackStorage::Segment(segment) = &mut pack.storage {
        segment.plaintext_bytes = bytes;
    }
    pack
}

#[test]
fn selection_surfaces_chain_errors_before_looking_for_a_pair() {
    let mut disconnected = main_pack(3, 3);
    disconnected.prerequisites = vec!["different".to_string()];
    let mut request_owned = main_pack(2, 2);
    request_owned.owner = GitPoolPackOwner::Request {
        request_id: "request".to_string(),
    };
    for (packs, expected) in [
        (
            vec![main_pack(1, 3)],
            GitCompactionError::NonGeometricCoverage {
                first_sequence: 1,
                last_sequence: 3,
                sequence_count: 3,
            },
        ),
        (
            vec![main_pack(2, 2), main_pack(3, 3)],
            GitCompactionError::InvalidStart { first_sequence: 2 },
        ),
        (
            vec![main_pack(1, 2), disconnected, main_pack(4, 4)],
            GitCompactionError::DisconnectedHistory {
                previous_head_oid: "head-2".to_string(),
                next_prerequisites: vec!["different".to_string()],
            },
        ),
        (
            vec![main_pack(1, 1), main_pack(3, 3)],
            GitCompactionError::NonContiguous {
                previous_last_sequence: 1,
                next_first_sequence: 3,
            },
        ),
        (
            vec![main_pack(1, 1), request_owned],
            GitCompactionError::NotMainPack {
                pack_id: "pack-2-2".to_string(),
            },
        ),
    ] {
        assert_eq!(
            GitCompactionPlan::select(&packs, u64::MAX).unwrap_err(),
            expected
        );
    }
}

#[test]
fn selection_chooses_the_oldest_pair_covering_equal_push_counts() {
    for (packs, selected) in [
        (vec![main_pack(1, 1), main_pack(2, 2)], Some(0..2)),
        (
            vec![main_pack(1, 4), main_pack(5, 5), main_pack(6, 6)],
            Some(1..3),
        ),
        (
            vec![
                main_pack(1, 4),
                main_pack(5, 6),
                main_pack(7, 8),
                main_pack(9, 9),
                main_pack(10, 10),
            ],
            Some(1..3),
        ),
        (
            vec![main_pack(1, 4), main_pack(5, 6), main_pack(7, 7)],
            None,
        ),
    ] {
        let plan = GitCompactionPlan::select(&packs, u64::MAX).unwrap();
        assert_eq!(
            plan.as_ref().map(|plan| plan.selected_packs().as_slice()),
            selected.map(|range| &packs[range])
        );
    }
}

#[test]
fn selection_skips_a_pair_over_the_byte_budget_and_counts_inline_packs() {
    let mut packs = vec![
        main_pack(1, 4),
        sized(main_pack(5, 6), 6),
        sized(main_pack(7, 8), 5),
        main_pack(9, 9),
        main_pack(10, 10),
    ];
    let plan = GitCompactionPlan::select(&packs, 10).unwrap().unwrap();
    assert_eq!(plan.selected_packs(), &packs[3..5]);

    packs[3].storage = GitPoolPackStorage::Inline {
        sealed: vec![0; 10],
    };
    assert_eq!(GitCompactionPlan::select(&packs, 10).unwrap(), None);
}

#[test]
fn replacement_covers_the_selected_range_and_keeps_its_boundaries() {
    let packs = [main_pack(1, 2), main_pack(3, 3), main_pack(4, 4)];
    let plan = GitCompactionPlan::select(&packs, u64::MAX)
        .unwrap()
        .unwrap();

    let replacement = plan.replacement(segment(3, 4)).unwrap();

    assert_eq!(
        replacement,
        GitPoolPack {
            id: "segment-3-4".to_string(),
            owner: GitPoolPackOwner::Main {
                first_sequence: 3,
                last_sequence: 4,
            },
            head_oid: "head-4".to_string(),
            prerequisites: vec!["head-2".to_string()],
            storage: GitPoolPackStorage::Segment(segment(3, 4)),
        }
    );
}

#[test]
fn replacement_validation_requires_exact_range_and_oid_boundaries() {
    let plan = GitCompactionPlan::select(&[main_pack(1, 4), main_pack(5, 8)], u64::MAX)
        .unwrap()
        .unwrap();
    let valid = plan.replacement(segment(1, 8)).unwrap();
    plan.validate_replacement(&valid).unwrap();

    let mut invalid = Vec::new();
    for owner in [
        GitPoolPackOwner::Main {
            first_sequence: 2,
            last_sequence: 8,
        },
        GitPoolPackOwner::Main {
            first_sequence: 1,
            last_sequence: 7,
        },
    ] {
        invalid.push(GitPoolPack {
            owner,
            ..valid.clone()
        });
    }
    invalid.push(GitPoolPack {
        prerequisites: vec!["different".to_string()],
        ..valid.clone()
    });
    invalid.push(GitPoolPack {
        head_oid: "different".to_string(),
        ..valid.clone()
    });
    invalid.push(GitPoolPack {
        id: "other".to_string(),
        ..valid
    });
    for replacement in invalid {
        assert_eq!(
            plan.validate_replacement(&replacement).unwrap_err(),
            GitCompactionError::ReplacementBoundaryMismatch
        );
    }
}

#[test]
fn repeatedly_selecting_pairs_settles_a_binary_frontier() {
    let mut packs = Vec::new();
    for sequence in 1..=1_024 {
        packs.push(main_pack(sequence, sequence));
        while let Some(plan) = GitCompactionPlan::select(&packs, u64::MAX).unwrap() {
            let [first, last] = plan.selected_packs();
            let start = packs.iter().position(|pack| pack.id == first.id).unwrap();
            let (first_sequence, _) = first.owner.main_sequences().unwrap();
            let (_, last_sequence) = last.owner.main_sequences().unwrap();
            let replacement = plan
                .replacement(segment(first_sequence, last_sequence))
                .unwrap();
            packs.splice(start..start + 2, [replacement]);
        }
        GitCompactionPlan::select(&packs, 0).unwrap();
    }
    assert_eq!(
        packs.last().unwrap().owner.main_sequences(),
        Some((1, 1_024))
    );
}
