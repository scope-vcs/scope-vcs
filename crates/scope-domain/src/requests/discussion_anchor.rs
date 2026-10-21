use super::validate_required;
use crate::{error::DomainError, policy::ScopePath};
use serde::{Deserialize, Serialize};

mod line_diff;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RequestDiffSide {
    Old,
    New,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LineSpan", into = "LineSpan")]
pub struct RequestLineRange {
    start_line: u32,
    end_line: u32,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct LineSpan {
    start_line: u32,
    end_line: u32,
}

impl TryFrom<LineSpan> for RequestLineRange {
    type Error = DomainError;

    fn try_from(span: LineSpan) -> Result<Self, Self::Error> {
        Self::new(span.start_line, span.end_line)
    }
}

impl From<RequestLineRange> for LineSpan {
    fn from(range: RequestLineRange) -> Self {
        Self {
            start_line: range.start_line,
            end_line: range.end_line,
        }
    }
}

impl RequestLineRange {
    pub fn new(start_line: u32, end_line: u32) -> Result<Self, DomainError> {
        if start_line == 0 {
            return Err(DomainError::invalid_input(
                "discussion line numbers start at 1",
            ));
        }
        if start_line > end_line {
            return Err(DomainError::invalid_input(
                "discussion start line must not be after its end line",
            ));
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }

    pub fn start_line(self) -> u32 {
        self.start_line
    }

    pub fn end_line(self) -> u32 {
        self.end_line
    }

    fn shifted_to(self, start_index: usize) -> Option<Self> {
        let start_line = u32::try_from(start_index.checked_add(1)?).ok()?;
        let end_line = start_line.checked_add(self.end_line - self.start_line)?;
        Some(Self {
            start_line,
            end_line,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionLines {
    pub side: RequestDiffSide,
    pub range: RequestLineRange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionAnchor {
    pub revision_id: String,
    pub commit_oid: Option<String>,
    pub path: Option<ScopePath>,
    pub lines: Option<RequestDiscussionLines>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestLineTarget<'a> {
    Missing,
    Binary,
    Text(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestLinePosition {
    Mapped(RequestLineRange),
    Outdated,
}

impl RequestDiscussionLines {
    pub fn ensure_target(self, target: RequestLineTarget<'_>) -> Result<(), DomainError> {
        let text = match target {
            RequestLineTarget::Missing => {
                return Err(DomainError::invalid_input(
                    "discussion file does not exist on the selected side",
                ));
            }
            RequestLineTarget::Binary => {
                return Err(DomainError::invalid_input(
                    "discussion lines require a text file",
                ));
            }
            RequestLineTarget::Text(text) => text,
        };
        if usize::try_from(self.range.end_line).is_ok_and(|end| end <= text_lines(text).len()) {
            Ok(())
        } else {
            Err(DomainError::invalid_input(
                "discussion lines are outside the file",
            ))
        }
    }
}

impl RequestDiscussionAnchor {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_required("revision id", &self.revision_id)?;
        if let Some(commit_oid) = self.commit_oid.as_deref() {
            validate_required("commit oid", commit_oid)?;
        }
        if self.lines.is_some() && self.path.is_none() {
            return Err(DomainError::invalid_input(
                "discussion line anchor requires a path",
            ));
        }
        Ok(())
    }
}

pub fn carry_forward_line_range(
    range: RequestLineRange,
    original: &str,
    newer: Option<&str>,
) -> RequestLinePosition {
    let Some(newer) = newer else {
        return RequestLinePosition::Outdated;
    };
    if original == newer {
        return RequestLinePosition::Mapped(range);
    }
    let old = text_lines(original);
    let new = text_lines(newer);
    let start = range.start_line as usize - 1;
    let end = range.end_line as usize;
    if end > old.len() {
        return RequestLinePosition::Outdated;
    }
    let mapped = line_diff::matched_new_index(&old, &new, start..end);
    match mapped.and_then(|start_index| range.shifted_to(start_index)) {
        Some(range) => RequestLinePosition::Mapped(range),
        None => RequestLinePosition::Outdated,
    }
}

fn text_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}
