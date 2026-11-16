use crate::error::DomainError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestackAncestry {
    pub head_contains_recorded: bool,
    pub head_contains_selected: bool,
    pub selected_contains_recorded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestackLayerBase {
    Recorded,
    Selected,
}

impl RestackLayerBase {
    pub fn locate(ancestry: RestackAncestry) -> Result<Self, DomainError> {
        let RestackAncestry {
            head_contains_recorded,
            head_contains_selected,
            selected_contains_recorded,
        } = ancestry;
        if head_contains_recorded && !selected_contains_recorded {
            Ok(Self::Recorded)
        } else if head_contains_selected {
            Ok(Self::Selected)
        } else if head_contains_recorded {
            Ok(Self::Recorded)
        } else {
            Err(DomainError::invalid_input(
                "the branch contains neither the request's recorded base nor the selected base; reset it to a layer on one of them, then restack",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
