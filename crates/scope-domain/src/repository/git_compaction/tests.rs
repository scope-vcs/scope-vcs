use super::*;

fn span(first_sequence: u64, last_sequence: u64, geometric_tier: u32) -> GitPackSpan {
    GitPackSpan {
        first_sequence,
        last_sequence,
        geometric_tier,
        base_oid: (first_sequence > 1).then(|| format!("head-{}", first_sequence - 1)),
        head_oid: format!("head-{last_sequence}"),
        segment: segment(first_sequence, last_sequence),
    }
}

fn segment(first_sequence: u64, last_sequence: u64) -> GitSegmentRef {
    GitSegmentRef {
        segment_id: format!("segment-{first_sequence}-{last_sequence}"),
        sha256: format!("{first_sequence:032x}{last_sequence:032x}"),
        plaintext_bytes: 1,
        encoding_version: 2,
    }
}

#[test]
fn selection_validates_the_layout_before_looking_for_a_pair() {
    let mut invalid = span(1, 1, 0);
    invalid.geometric_tier = 1;

    assert!(matches!(
        GitCompactionPlan::select(&[invalid], u64::MAX),
        Err(GitCompactionError::InvalidLayout(
            GitPackLayoutError::InvalidGeometricTier { .. }
        ))
    ));
}

#[test]
fn selection_rejects_missing_or_disconnected_predecessor_history() {
    let missing_prefix = [span(2, 2, 0), span(3, 3, 0)];
    assert_eq!(
        GitCompactionPlan::select(&missing_prefix, u64::MAX).unwrap_err(),
        GitCompactionError::InvalidLayout(GitPackLayoutError::InvalidStart { first_sequence: 2 })
    );

    let disconnected = [
        span(1, 2, 1),
        GitPackSpan {
            base_oid: Some("different".to_string()),
            ..span(3, 3, 0)
        },
        span(4, 4, 0),
    ];
    assert!(matches!(
        GitCompactionPlan::select(&disconnected, u64::MAX),
        Err(GitCompactionError::InvalidLayout(
            GitPackLayoutError::DisconnectedHistory { .. }
        ))
    ));
}

#[test]
fn selection_rejects_a_gap_before_looking_for_a_pair() {
    let spans = [span(1, 1, 0), span(3, 3, 0)];

    assert_eq!(
        GitCompactionPlan::select(&spans, u64::MAX).unwrap_err(),
        GitCompactionError::InvalidLayout(GitPackLayoutError::NonContiguous {
            previous_last_sequence: 1,
            next_first_sequence: 3,
        })
    );
}

#[test]
fn selection_allows_the_newest_pair() {
    let spans = [span(1, 4, 2), span(5, 5, 0), span(6, 6, 0)];

    let plan = GitCompactionPlan::select(&spans, u64::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(
        plan.selected_spans()
            .iter()
            .map(|span| (span.first_sequence, span.last_sequence))
            .collect::<Vec<_>>(),
        [(5, 5), (6, 6)]
    );
}

#[test]
fn selection_chooses_the_oldest_equal_tier_pair() {
    let spans = [
        span(1, 4, 2),
        span(5, 6, 1),
        span(7, 8, 1),
        span(9, 9, 0),
        span(10, 10, 0),
    ];

    let plan = GitCompactionPlan::select(&spans, u64::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(plan.selected_spans(), &spans[1..3]);

    let mut first_pair_over_budget = spans.clone();
    first_pair_over_budget[1].segment.plaintext_bytes = 6;
    first_pair_over_budget[2].segment.plaintext_bytes = 5;
    let plan = GitCompactionPlan::select(&first_pair_over_budget, 10)
        .unwrap()
        .unwrap();
    assert_eq!(plan.selected_spans(), &first_pair_over_budget[3..5]);
}

#[test]
fn selection_returns_none_when_no_equal_tier_pair_exists() {
    let spans = [span(1, 4, 2), span(5, 6, 1), span(7, 7, 0)];

    assert_eq!(GitCompactionPlan::select(&spans, u64::MAX).unwrap(), None);
}

#[test]
fn a_prefix_plan_selects_the_pair() {
    let spans = [span(1, 1, 0), span(2, 2, 0)];

    let plan = GitCompactionPlan::select(&spans, u64::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(plan.selected_spans(), &spans);
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
