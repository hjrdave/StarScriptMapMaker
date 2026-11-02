//! The icon each step of the History panel shows beside its name.

use crate::icons::Icon;

/// The icon of the history step named `label` (a Command's `label()`, a tool's gesture name, or
/// "Original"). `None` is a step with no icon of its own yet; the list draws a dot for it.
pub(super) fn step_icon(label: &str) -> Option<Icon> {
    Some(match label {
        "Original" => Icon::Reset,
        "Paint Land" | "Create Landmass" | "Add Region" | "Subtract Region" => Icon::Landmass,
        "Paint Lake" | "Paint Water" | "Erase Water" | "Create Lake" => Icon::Lake,
        "Add River" | "Edit River" => Icon::River,
        "Clip to Landmass" | "Clear Clip" => Icon::Clip,
        "Lock" | "Change Lock" | "Change Move Lock" => Icon::Lock,
        "Union" => Icon::Union,
        "Subtract" => Icon::Subtract,
        "Split Apart" => Icon::SplitApart,
        "Move Points" | "Add Point" | "Delete Points" => Icon::Nodes,
        "Add Stamp" | "Edit Stamp" | "Place Stamps" => Icon::Stamp,
        "Add Label" | "Edit Label" => Icon::Label,
        "Create Layer" | "Rename Layer" => Icon::Layer,
        "Group" | "Ungroup" => Icon::LayerGroup,
        "Link" | "Unlink" => Icon::Link,
        "Change Visibility" => Icon::Visible,
        "Change Depth Sort" => Icon::DepthSort,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_has_the_reset_icon() {
        assert_eq!(step_icon("Original"), Some(Icon::Reset));
    }

    #[test]
    fn actions_with_an_icon_of_their_own_use_it() {
        assert_eq!(step_icon("Paint Land"), Some(Icon::Landmass));
        assert_eq!(step_icon("Clip to Landmass"), Some(Icon::Clip));
        assert_eq!(step_icon("Group"), Some(Icon::LayerGroup));
    }

    #[test]
    fn actions_without_an_icon_get_none() {
        assert_eq!(step_icon("Change Opacity"), None);
        assert_eq!(step_icon("Resize Canvas"), None);
    }
}
