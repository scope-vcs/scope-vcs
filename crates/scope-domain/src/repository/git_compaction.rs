use super::git::{
    GitPackLayoutError, GitPackSpan, GitSegmentRef, validate_git_pack_layout,
    validate_git_pack_span_run,
};

/// A compaction decision selected from a fully valid Git pack layout.
///
/// The selected run always contains exactly two adjacent equal-tier spans. Its
/// optional predecessor is the immediately preceding span, so the boundary
/// accessors are infallible and agree with the validated layout history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitCompactionPlan {
    predecessor: Option<GitPackSpan>,
    selected_spans: [GitPackSpan; 2],
}

impl GitCompactionPlan {
    /// Validates the complete layout before applying the count threshold and
    /// selecting its oldest adjacent equal-tier pair.
    pub fn select(
        spans: &[GitPackSpan],
        minimum_spans: u64,
    ) -> Result<Option<Self>, GitCompactionError> {
        validate_minimum_spans(minimum_spans)?;
        validate_git_pack_layout(spans)?;
        if u64::try_from(spans.len()).unwrap_or(u64::MAX) < minimum_spans {
            return Ok(None);
        }

        let Some(pair_start) = spans.windows(2).position(|pair| {
            pair[0].geometric_tier == pair[1].geometric_tier
                && pair[0].last_sequence.checked_add(1) == Some(pair[1].first_sequence)
        }) else {
            return Ok(None);
        };

        Ok(Some(Self {
            predecessor: pair_start
                .checked_sub(1)
                .and_then(|index| spans.get(index))
                .cloned(),
            selected_spans: [spans[pair_start].clone(), spans[pair_start + 1].clone()],
        }))
    }

    pub fn selected_spans(&self) -> &[GitPackSpan; 2] {
        &self.selected_spans
    }

    pub fn predecessor(&self) -> Option<&GitPackSpan> {
        self.predecessor.as_ref()
    }

    pub fn base_oid(&self) -> Option<&str> {
        self.selected_spans[0].base_oid.as_deref()
    }

    pub fn head_oid(&self) -> &str {
        &self.selected_spans[1].head_oid
    }

    pub fn replacement(&self, segment: GitSegmentRef) -> Result<GitPackSpan, GitCompactionError> {
        let first = &self.selected_spans[0];
        let last = &self.selected_spans[1];
        let mut replacement = GitPackSpan {
            first_sequence: first.first_sequence,
            last_sequence: last.last_sequence,
            geometric_tier: 0,
            base_oid: first.base_oid.clone(),
            head_oid: last.head_oid.clone(),
            segment,
        };
        replacement.geometric_tier = replacement.expected_geometric_tier()?;
        self.validate_replacement(&replacement)?;
        Ok(replacement)
    }

    pub fn validate_replacement(
        &self,
        replacement: &GitPackSpan,
    ) -> Result<(), GitCompactionError> {
        validate_git_compaction_replacement(&self.selected_spans, replacement)
    }

    pub fn resulting_layout(
        &self,
        current: &[GitPackSpan],
        range_start: usize,
        replacement: GitPackSpan,
    ) -> Result<Vec<GitPackSpan>, GitCompactionError> {
        let range_end = range_start + self.selected_spans.len();
        debug_assert_eq!(
            current.get(range_start..range_end),
            Some(self.selected_spans.as_slice()),
            "the caller must match the selected spans under its persistence lock"
        );

        let mut resulting_layout = Vec::with_capacity(current.len().saturating_sub(1));
        resulting_layout.extend(current[..range_start].iter().cloned());
        resulting_layout.push(replacement);
        resulting_layout.extend(current[range_end..].iter().cloned());
        validate_git_pack_layout(&resulting_layout)?;
        Ok(resulting_layout)
    }
}

pub fn validate_minimum_spans(minimum_spans: u64) -> Result<(), GitCompactionError> {
    if minimum_spans < 2 {
        return Err(GitCompactionError::MinimumSpansTooSmall);
    }
    Ok(())
}

fn validate_git_compaction_replacement(
    selected_spans: &[GitPackSpan],
    replacement: &GitPackSpan,
) -> Result<(), GitCompactionError> {
    if selected_spans.len() != 2 {
        return Err(GitCompactionError::InvalidSelectedSpanCount);
    }
    validate_git_pack_span_run(selected_spans)?;
    let first = &selected_spans[0];
    let last = &selected_spans[1];
    if replacement.first_sequence != first.first_sequence
        || replacement.last_sequence != last.last_sequence
        || replacement.base_oid != first.base_oid
        || replacement.head_oid != last.head_oid
    {
        return Err(GitCompactionError::ReplacementBoundaryMismatch);
    }
    if first.geometric_tier != last.geometric_tier {
        return Err(GitCompactionError::SelectedTiersDiffer);
    }
    let expected_tier = replacement.expected_geometric_tier()?;
    if replacement.geometric_tier != expected_tier {
        return Err(GitCompactionError::InvalidReplacementTier { expected_tier });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GitCompactionError {
    #[error("Git compaction span threshold must be at least 2")]
    MinimumSpansTooSmall,
    #[error("Git compaction requires exactly two expected pack spans")]
    InvalidSelectedSpanCount,
    #[error(transparent)]
    InvalidLayout(#[from] GitPackLayoutError),
    #[error("Git compaction replacement must cover exactly the selected pack spans")]
    ReplacementBoundaryMismatch,
    #[error("Git compaction requires adjacent pack spans from the same tier")]
    SelectedTiersDiffer,
    #[error("Git compaction replacement tier must be {expected_tier}")]
    InvalidReplacementTier { expected_tier: u32 },
}

#[cfg(test)]
mod tests;
