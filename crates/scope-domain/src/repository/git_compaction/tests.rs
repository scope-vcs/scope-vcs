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
fn minimum_span_threshold_requires_two_spans() {
    assert_eq!(
        validate_minimum_spans(1).unwrap_err().to_string(),
        "Git compaction span threshold must be at least 2"
    );
    validate_minimum_spans(2).unwrap();
}

#[test]
fn selection_validates_the_layout_before_count_eligibility() {
    let mut invalid = span(1, 1, 0);
    invalid.geometric_tier = 1;

    assert!(matches!(
        GitCompactionPlan::select(&[invalid], 2),
        Err(GitCompactionError::InvalidLayout(
            GitPackLayoutError::InvalidGeometricTier { .. }
        ))
    ));
}

#[test]
fn selection_rejects_missing_or_disconnected_predecessor_history() {
    let missing_prefix = [span(2, 2, 0), span(3, 3, 0)];
    assert_eq!(
        GitCompactionPlan::select(&missing_prefix, 2).unwrap_err(),
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
        GitCompactionPlan::select(&disconnected, 3),
        Err(GitCompactionError::InvalidLayout(
            GitPackLayoutError::DisconnectedHistory { .. }
        ))
    ));
}

#[test]
fn selection_rejects_a_gap_before_looking_for_a_pair() {
    let spans = [span(1, 1, 0), span(3, 3, 0)];

    assert_eq!(
        GitCompactionPlan::select(&spans, 2).unwrap_err(),
        GitCompactionError::InvalidLayout(GitPackLayoutError::NonContiguous {
            previous_last_sequence: 1,
            next_first_sequence: 3,
        })
    );
}

#[test]
fn selection_observes_the_threshold_and_allows_the_newest_span() {
    let spans = [span(1, 4, 2), span(5, 5, 0), span(6, 6, 0)];

    assert_eq!(GitCompactionPlan::select(&spans, 4).unwrap(), None);
    let plan = GitCompactionPlan::select(&spans, 3).unwrap().unwrap();
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

    let plan = GitCompactionPlan::select(&spans, 3).unwrap().unwrap();
    assert_eq!(plan.selected_spans(), &spans[1..3]);
    assert_eq!(plan.predecessor(), Some(&spans[0]));
    assert_eq!(plan.base_oid(), Some("head-4"));
    assert_eq!(plan.head_oid(), "head-8");
}

#[test]
fn selection_returns_none_when_no_equal_tier_pair_exists() {
    let spans = [span(1, 4, 2), span(5, 6, 1), span(7, 7, 0)];

    assert_eq!(GitCompactionPlan::select(&spans, 2).unwrap(), None);
}

#[test]
fn a_prefix_plan_has_no_predecessor_or_base() {
    let spans = [span(1, 1, 0), span(2, 2, 0)];

    let plan = GitCompactionPlan::select(&spans, 2).unwrap().unwrap();
    assert_eq!(plan.predecessor(), None);
    assert_eq!(plan.base_oid(), None);
    assert_eq!(plan.head_oid(), "head-2");
}

#[test]
fn replacement_uses_the_selected_range_and_oid_boundaries() {
    let spans = [span(1, 2, 1), span(3, 3, 0), span(4, 4, 0)];
    let plan = GitCompactionPlan::select(&spans, 3).unwrap().unwrap();
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
    let plan = GitCompactionPlan::select(&current[..3], 3)
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
fn binary_frontier_advances_past_power_of_two_boundaries_with_a_fixed_limit() {
    let mut spans = Vec::new();
    for sequence in 1..=1_024 {
        assert!(spans.len() < 64, "push capacity deadlocked at {sequence}");
        spans.push(span(sequence, sequence, 0));
        if spans.len() >= 32 {
            let plan = GitCompactionPlan::select(&spans, 32)
                .unwrap()
                .expect("a full descending binary frontier has a mergeable pair");
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
