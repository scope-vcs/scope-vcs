use super::*;
use crate::repository::git::fixtures::{segment, span};

#[test]
fn selection_surfaces_layout_errors_before_looking_for_a_pair() {
    let disconnected = GitPackSpan {
        base_oid: Some("different".to_string()),
        ..span(3, 3, 0)
    };
    for (spans, expected) in [
        (
            vec![GitPackSpan {
                geometric_tier: 1,
                ..span(1, 1, 0)
            }],
            GitPackLayoutError::InvalidGeometricTier {
                first_sequence: 1,
                last_sequence: 1,
                actual: 1,
                expected: 0,
            },
        ),
        (
            vec![span(2, 2, 0), span(3, 3, 0)],
            GitPackLayoutError::InvalidStart { first_sequence: 2 },
        ),
        (
            vec![span(1, 2, 1), disconnected, span(4, 4, 0)],
            GitPackLayoutError::DisconnectedHistory {
                previous_head_oid: "head-2".to_string(),
                next_base_oid: Some("different".to_string()),
            },
        ),
        (
            vec![span(1, 1, 0), span(3, 3, 0)],
            GitPackLayoutError::NonContiguous {
                previous_last_sequence: 1,
                next_first_sequence: 3,
            },
        ),
    ] {
        assert_eq!(
            GitCompactionPlan::select(&spans, u64::MAX).unwrap_err(),
            GitCompactionError::InvalidLayout(expected)
        );
    }
}

#[test]
fn selection_chooses_the_oldest_equal_tier_pair() {
    for (spans, selected) in [
        (vec![span(1, 1, 0), span(2, 2, 0)], Some(0..2)),
        (
            vec![span(1, 4, 2), span(5, 5, 0), span(6, 6, 0)],
            Some(1..3),
        ),
        (
            vec![
                span(1, 4, 2),
                span(5, 6, 1),
                span(7, 8, 1),
                span(9, 9, 0),
                span(10, 10, 0),
            ],
            Some(1..3),
        ),
        (vec![span(1, 4, 2), span(5, 6, 1), span(7, 7, 0)], None),
    ] {
        let plan = GitCompactionPlan::select(&spans, u64::MAX).unwrap();
        assert_eq!(
            plan.as_ref().map(|plan| plan.selected_spans().as_slice()),
            selected.map(|range| &spans[range])
        );
    }
}

#[test]
fn selection_skips_a_pair_over_the_byte_budget() {
    let mut spans = [
        span(1, 4, 2),
        span(5, 6, 1),
        span(7, 8, 1),
        span(9, 9, 0),
        span(10, 10, 0),
    ];
    spans[1].segment.plaintext_bytes = 6;
    spans[2].segment.plaintext_bytes = 5;
    let plan = GitCompactionPlan::select(&spans, 10).unwrap().unwrap();
    assert_eq!(plan.selected_spans(), &spans[3..5]);
}

#[test]
fn replacement_uses_the_selected_range_and_oid_boundaries() {
    let spans = [span(1, 2, 1), span(3, 3, 0), span(4, 4, 0)];
    let plan = GitCompactionPlan::select(&spans, u64::MAX)
        .unwrap()
        .unwrap();
    let replacement_segment = segment(3, 4);

    let replacement = plan.replacement(replacement_segment.clone()).unwrap();

    assert_eq!(
        replacement,
        GitPackSpan {
            first_sequence: 3,
            last_sequence: 4,
            geometric_tier: 1,
            base_oid: Some("head-2".to_string()),
            head_oid: "head-4".to_string(),
            segment: replacement_segment,
        }
    );
}

#[test]
fn raw_replacement_validation_preserves_error_precedence() {
    assert_eq!(
        validate_git_compaction_replacement(&[], &span(1, 2, 1))
            .unwrap_err()
            .to_string(),
        "Git compaction requires exactly two expected pack spans"
    );

    let malformed = [GitPackSpan {
        geometric_tier: 4,
        ..span(1, 1, 0)
    }];
    let wrong = span(10, 20, 0);
    assert_eq!(
        validate_git_compaction_replacement(&malformed, &wrong)
            .unwrap_err()
            .to_string(),
        "Git compaction requires exactly two expected pack spans"
    );

    let malformed = [
        span(1, 2, 1),
        GitPackSpan {
            base_oid: Some("disconnected".to_string()),
            ..span(3, 4, 1)
        },
    ];
    assert!(matches!(
        validate_git_compaction_replacement(&malformed, &wrong),
        Err(GitCompactionError::InvalidLayout(
            GitPackLayoutError::DisconnectedHistory { .. }
        ))
    ));

    let unequal_tiers = [span(1, 4, 2), span(5, 6, 1)];
    assert_eq!(
        validate_git_compaction_replacement(&unequal_tiers, &wrong)
            .unwrap_err()
            .to_string(),
        "Git compaction replacement must cover exactly the selected pack spans"
    );
    let exact_non_geometric = GitPackSpan {
        first_sequence: 1,
        last_sequence: 6,
        geometric_tier: 0,
        base_oid: None,
        head_oid: "head-6".to_string(),
        segment: segment(1, 6),
    };
    assert_eq!(
        validate_git_compaction_replacement(&unequal_tiers, &exact_non_geometric)
            .unwrap_err()
            .to_string(),
        "Git compaction requires adjacent pack spans from the same tier"
    );
}

#[test]
fn raw_replacement_validation_rejects_sequence_overflow() {
    let selected = [
        span(u64::MAX, u64::MAX, 0),
        GitPackSpan {
            base_oid: Some("head-max".to_string()),
            ..span(0, 0, 0)
        },
    ];

    assert_eq!(
        validate_git_compaction_replacement(&selected, &span(1, 2, 1)).unwrap_err(),
        GitCompactionError::InvalidLayout(GitPackLayoutError::InvalidRange {
            first_sequence: u64::MAX,
            last_sequence: u64::MAX,
        })
    );
}

#[test]
fn replacement_validation_requires_exact_range_and_oid_boundaries() {
    let selected = [span(1, 4, 2), span(5, 8, 2)];
    let valid = span(1, 8, 3);
    validate_git_compaction_replacement(&selected, &valid).unwrap();

    let invalid_replacements = [
        GitPackSpan {
            first_sequence: 2,
            ..valid.clone()
        },
        GitPackSpan {
            last_sequence: 7,
            ..valid.clone()
        },
        GitPackSpan {
            base_oid: Some("different".to_string()),
            ..valid.clone()
        },
        GitPackSpan {
            head_oid: "different".to_string(),
            ..valid
        },
    ];
    for replacement in invalid_replacements {
        assert_eq!(
            validate_git_compaction_replacement(&selected, &replacement)
                .unwrap_err()
                .to_string(),
            "Git compaction replacement must cover exactly the selected pack spans"
        );
    }
}

#[test]
fn replacement_validation_requires_the_derived_tier() {
    let selected = [span(1, 4, 2), span(5, 8, 2)];
    let replacement = span(1, 8, 2);

    assert_eq!(
        validate_git_compaction_replacement(&selected, &replacement)
            .unwrap_err()
            .to_string(),
        "Git compaction replacement tier must be 3"
    );
}

#[test]
fn resulting_layout_replaces_only_the_selected_pair() {
    let current = [span(1, 2, 1), span(3, 3, 0), span(4, 4, 0), span(5, 5, 0)];
    let plan = GitCompactionPlan::select(&current[..3], u64::MAX)
        .unwrap()
        .unwrap();
    let replacement = plan.replacement(segment(3, 4)).unwrap();

    let resulting = plan.resulting_layout(&current, 1, replacement).unwrap();

    assert_eq!(
        resulting
            .iter()
            .map(|span| (span.first_sequence, span.last_sequence))
            .collect::<Vec<_>>(),
        [(1, 2), (3, 4), (5, 5)]
    );
    validate_git_pack_layout(&resulting).unwrap();
}

#[test]
fn repeatedly_selecting_pairs_settles_a_binary_frontier() {
    let mut spans = Vec::new();
    for sequence in 1..=1_024 {
        spans.push(span(sequence, sequence, 0));
        while let Some(plan) = GitCompactionPlan::select(&spans, u64::MAX).unwrap() {
            let range_start = spans
                .iter()
                .position(|span| span.first_sequence == plan.selected_spans()[0].first_sequence)
                .unwrap();
            let replacement = plan
                .replacement(segment(
                    plan.selected_spans()[0].first_sequence,
                    plan.selected_spans()[1].last_sequence,
                ))
                .unwrap();
            spans = plan
                .resulting_layout(&spans, range_start, replacement)
                .unwrap();
        }
        validate_git_pack_layout(&spans).unwrap();
    }
    assert_eq!(spans.last().unwrap().last_sequence, 1_024);
}
