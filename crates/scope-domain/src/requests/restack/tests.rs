use super::*;
use crate::error::DomainErrorKind;

fn locate(
    head_contains_recorded: bool,
    head_contains_selected: bool,
    selected_contains_recorded: bool,
) -> Result<RestackLayerBase, DomainError> {
    RestackLayerBase::locate(RestackAncestry {
        head_contains_recorded,
        head_contains_selected,
        selected_contains_recorded,
    })
}

#[test]
fn a_layer_already_rebased_onto_a_rewritten_parent_needs_no_replay() {
    assert_eq!(locate(false, true, false), Ok(RestackLayerBase::Selected));
}

#[test]
fn a_layer_already_on_an_extended_or_unchanged_base_needs_no_replay() {
    assert_eq!(locate(true, true, true), Ok(RestackLayerBase::Selected));
}

#[test]
fn a_layer_on_the_recorded_base_replays_from_it() {
    assert_eq!(locate(true, false, false), Ok(RestackLayerBase::Recorded));
    assert_eq!(locate(true, false, true), Ok(RestackLayerBase::Recorded));
}

#[test]
fn a_layer_moving_onto_an_ancestor_of_its_recorded_base_replays_only_its_own_commits() {
    assert_eq!(locate(true, true, false), Ok(RestackLayerBase::Recorded));
}

#[test]
fn a_layer_on_neither_base_is_refused() {
    for selected_contains_recorded in [false, true] {
        let refused = locate(false, false, selected_contains_recorded).unwrap_err();
        assert_eq!(refused.kind, DomainErrorKind::InvalidInput);
    }
}
