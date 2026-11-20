//! A list shown beside an editor that keeps the keyboard focus, as an AccessKit list box.
//!
//! A completion list or a command palette opened over an editor leaves the focus in the
//! editor, so the caret stays visible and typing still edits or filters. A screen reader
//! follows the focus, though, so it would never read the list. This follows the WAI-ARIA
//! editable combobox pattern: the editor's node gains the list as a `ListBox` child, says
//! it controls that list, and names the highlighted option as its active descendant. The
//! AccessKit adapters then report that option as focused while the editor keeps focus.
use gpui::{SharedString, accesskit};

/// A list the editor shows while it keeps the keyboard focus, given to screen readers.
///
/// Set with [`InputBaseState::set_accessibility_popup`](super::InputBaseState::set_accessibility_popup)
/// while the list is open and cleared when it closes. The editor projects it only while
/// it has an [`InputAccessibility`](super::InputAccessibility).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputAccessibilityPopup {
    label: Option<SharedString>,
    options: Vec<InputAccessibilityOption>,
    active: Option<usize>,
}

impl InputAccessibilityPopup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_label(mut self, label: Option<SharedString>) -> Self {
        self.label = label;
        self
    }

    pub fn with_options(mut self, options: Vec<InputAccessibilityOption>) -> Self {
        self.options = options;
        self
    }

    /// The highlighted option's index; an index past the end highlights none.
    pub fn with_active(mut self, active: Option<usize>) -> Self {
        self.active = active;
        self
    }

    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    pub fn options(&self) -> &[InputAccessibilityOption] {
        &self.options
    }

    pub fn active(&self) -> Option<usize> {
        self.active.filter(|&index| index < self.options.len())
    }
}

/// One option of an [`InputAccessibilityPopup`].
#[derive(Clone, Debug, PartialEq)]
pub struct InputAccessibilityOption {
    name: SharedString,
    description: Option<SharedString>,
    shortcut: Option<SharedString>,
}

impl InputAccessibilityOption {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            description: None,
            shortcut: None,
        }
    }

    pub fn with_description(mut self, description: Option<SharedString>) -> Self {
        self.description = description;
        self
    }

    /// The option's key binding, as a screen reader should speak it, such as "Ctrl+S".
    pub fn with_shortcut(mut self, shortcut: Option<SharedString>) -> Self {
        self.shortcut = shortcut;
        self
    }

    pub fn name(&self) -> &SharedString {
        &self.name
    }

    pub fn description(&self) -> Option<&SharedString> {
        self.description.as_ref()
    }

    pub fn shortcut(&self) -> Option<&SharedString> {
        self.shortcut.as_ref()
    }
}

impl InputAccessibilityPopup {
    pub(super) fn push(&self, builder: &mut gpui::A11ySubtreeBuilder) {
        let list_id = builder.synthetic_node_id("popup");
        let option_ids: Vec<_> = (0..self.options.len())
            .map(|index| builder.synthetic_node_id(("popup-option", index)))
            .collect();
        let (list, options) = self.nodes(&option_ids);
        builder.push_child(list_id, list);
        // A synthetic child always joins the editor's node, so each option is pushed there
        // and then handed to the list.
        for (&id, option) in option_ids.iter().zip(options) {
            builder.push_child(id, option);
        }
        self.describe_owner(builder.parent_node(), list_id, &option_ids);
    }

    /// The list's node and its options' nodes, the list holding `option_ids` as children.
    fn nodes(&self, option_ids: &[accesskit::NodeId]) -> (accesskit::Node, Vec<accesskit::Node>) {
        let mut list = accesskit::Node::new(accesskit::Role::ListBox);
        if let Some(label) = &self.label {
            list.set_label(label.to_string());
        }
        list.set_children(option_ids.to_vec());
        let active = self.active();
        let size = self.options.len();
        let options = self
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let mut node = accesskit::Node::new(accesskit::Role::ListBoxOption);
                node.set_label(option.name.to_string());
                if let Some(description) = &option.description {
                    node.set_description(description.to_string());
                }
                if let Some(shortcut) = &option.shortcut {
                    node.set_keyboard_shortcut(shortcut.to_string());
                }
                node.set_position_in_set(index + 1);
                node.set_size_of_set(size);
                node.set_selected(active == Some(index));
                node
            })
            .collect();
        (list, options)
    }

    /// Point the editor's node at the list and its highlighted option.
    fn describe_owner(
        &self,
        owner: &mut accesskit::Node,
        list_id: accesskit::NodeId,
        option_ids: &[accesskit::NodeId],
    ) {
        let children: Vec<_> = owner
            .children()
            .iter()
            .copied()
            .filter(|id| !option_ids.contains(id))
            .collect();
        owner.set_children(children);
        owner.set_has_popup(accesskit::HasPopup::Listbox);
        owner.set_expanded(true);
        owner.set_controls(vec![list_id]);
        if let Some(active) = self.active() {
            owner.set_active_descendant(option_ids[active]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::NodeId;

    fn popup(active: Option<usize>) -> InputAccessibilityPopup {
        InputAccessibilityPopup::new()
            .with_label(Some("Commands".into()))
            .with_options(vec![
                InputAccessibilityOption::new("Save")
                    .with_description(Some("File".into()))
                    .with_shortcut(Some("Ctrl+S".into())),
                InputAccessibilityOption::new("Quit"),
            ])
            .with_active(active)
    }

    const IDS: [NodeId; 2] = [NodeId(10), NodeId(11)];

    #[test]
    fn the_list_holds_its_options_in_order() {
        let (list, options) = popup(Some(1)).nodes(&IDS);
        assert_eq!(list.role(), accesskit::Role::ListBox);
        assert_eq!(list.label(), Some("Commands"));
        assert_eq!(list.children(), IDS);
        assert_eq!(options[0].label(), Some("Save"));
        assert_eq!(options[0].description(), Some("File"));
        assert_eq!(options[0].keyboard_shortcut(), Some("Ctrl+S"));
        assert_eq!(options[0].position_in_set(), Some(1));
        assert_eq!(options[1].size_of_set(), Some(2));
        assert_eq!(options[0].is_selected(), Some(false));
        assert_eq!(options[1].is_selected(), Some(true));
    }

    #[test]
    fn the_editor_owns_the_list_and_points_at_the_active_option() {
        let mut owner = accesskit::Node::new(accesskit::Role::MultilineTextInput);
        let run = NodeId(1);
        let list = NodeId(9);
        owner.set_children(vec![run, list, IDS[0], IDS[1]]);
        popup(Some(1)).describe_owner(&mut owner, list, &IDS);
        assert_eq!(owner.children(), [run, list]);
        assert_eq!(owner.controls(), [list]);
        assert_eq!(owner.is_expanded(), Some(true));
        assert_eq!(owner.has_popup(), Some(accesskit::HasPopup::Listbox));
        assert_eq!(owner.active_descendant(), Some(IDS[1]));
    }

    #[test]
    fn an_active_index_past_the_end_highlights_nothing() {
        let mut owner = accesskit::Node::new(accesskit::Role::MultilineTextInput);
        popup(Some(5)).describe_owner(&mut owner, NodeId(9), &IDS);
        assert_eq!(owner.active_descendant(), None);
        let (_, options) = popup(Some(5)).nodes(&IDS);
        assert!(
            options
                .iter()
                .all(|option| option.is_selected() == Some(false))
        );
    }
}
