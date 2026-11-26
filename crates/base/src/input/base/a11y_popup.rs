//! A list shown beside an element that keeps the keyboard focus, as an AccessKit list box.
//!
//! A completion list or a command palette opened over an editor leaves the focus in the
//! editor, so the caret stays visible and typing still edits or filters. A screen reader
//! follows the focus, though, so it would never read the list. This follows the WAI-ARIA
//! editable combobox pattern: the focused element's node gains the list as a `ListBox`
//! child, says it controls that list, and names the highlighted option as its active
//! descendant. The AccessKit adapters then report that option as focused while the element
//! keeps focus. An editor does this itself; any other focused element can with [`push`].
//!
//! [`push`]: InputAccessibilityPopup::push
use gpui::{Bounds, Pixels, SharedString, accesskit};

/// A list shown while an editor or another element keeps the keyboard focus, given to
/// screen readers.
///
/// On an editor, set it with
/// [`InputBaseState::set_accessibility_popup`](super::InputBaseState::set_accessibility_popup)
/// while the list is open and clear it when it closes. On another focusable element, call
/// [`Self::push`] from the element's synthetic children. An editor projects it only while
/// it has an [`InputAccessibility`](super::InputAccessibility); without one the popup is
/// kept but not shown to screen readers. The options have no bounds of their own; give
/// the list's with [`Self::with_bounds`] so a magnifier can follow it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputAccessibilityPopup {
    label: Option<SharedString>,
    options: Vec<InputAccessibilityOption>,
    active: Option<usize>,
    bounds: Option<Bounds<Pixels>>,
}

impl InputAccessibilityPopup {
    /// An empty list with no label and nothing highlighted.
    pub fn new() -> Self {
        Self::default()
    }

    /// The list's name, as a screen reader announces it.
    pub fn with_label(mut self, label: Option<SharedString>) -> Self {
        self.label = label;
        self
    }

    /// The list's options, in the order shown.
    pub fn with_options(mut self, options: Vec<InputAccessibilityOption>) -> Self {
        self.options = options;
        self
    }

    /// The highlighted option's index; an index past the end highlights none.
    pub fn with_active(mut self, active: Option<usize>) -> Self {
        self.active = active;
        self
    }

    /// Where the list is drawn, in window coordinates. It is a snapshot: set the popup
    /// again when the list moves or resizes.
    pub fn with_bounds(mut self, bounds: Option<Bounds<Pixels>>) -> Self {
        self.bounds = bounds;
        self
    }

    /// The list's name.
    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    /// The list's options, in the order shown.
    pub fn options(&self) -> &[InputAccessibilityOption] {
        &self.options
    }

    /// The highlighted option's index, or None when none is or the index is past the end.
    pub fn active(&self) -> Option<usize> {
        self.active.filter(|&index| index < self.options.len())
    }

    /// Where the list is drawn, in window coordinates.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
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
    /// An option named `name`, as a screen reader announces it.
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            description: None,
            shortcut: None,
        }
    }

    /// More about the option, announced after its name, such as a file's folder.
    pub fn with_description(mut self, description: Option<SharedString>) -> Self {
        self.description = description;
        self
    }

    /// The option's key binding, as a screen reader should speak it, such as "Ctrl+S".
    pub fn with_shortcut(mut self, shortcut: Option<SharedString>) -> Self {
        self.shortcut = shortcut;
        self
    }

    /// The option's name.
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    /// More about the option, announced after its name.
    pub fn description(&self) -> Option<&SharedString> {
        self.description.as_ref()
    }

    /// The option's key binding, as a screen reader speaks it.
    pub fn shortcut(&self) -> Option<&SharedString> {
        self.shortcut.as_ref()
    }
}

impl InputAccessibilityPopup {
    /// Adds the list under the node `builder` builds children for, and points that node at
    /// the list and its highlighted option. Call it from
    /// [`a11y_synthetic_children`](gpui::StatefulInteractiveElement::a11y_synthetic_children)
    /// of the element holding the keyboard focus, which needs an id and a role to have a
    /// node. `scale` is the window's scale factor: AccessKit's bounds are in physical pixels.
    pub fn push(&self, builder: &mut gpui::A11ySubtreeBuilder, scale: f32) {
        let list_id = builder.synthetic_node_id("popup");
        let option_ids: Vec<_> = (0..self.options.len())
            .map(|index| builder.synthetic_node_id(("popup-option", index)))
            .collect();
        let (list, options) = self.nodes(&option_ids, scale);
        builder.push_child(list_id, list);
        // A synthetic child always joins the editor's node, so each option is pushed there
        // and then handed to the list.
        for (&id, option) in option_ids.iter().zip(options) {
            builder.push_child(id, option);
        }
        self.describe_owner(builder.parent_node(), list_id, &option_ids);
    }

    /// The list's node and its options' nodes, the list holding `option_ids` as children.
    fn nodes(
        &self,
        option_ids: &[accesskit::NodeId],
        scale: f32,
    ) -> (accesskit::Node, Vec<accesskit::Node>) {
        let mut list = accesskit::Node::new(accesskit::Role::ListBox);
        if let Some(bounds) = self.bounds {
            list.set_bounds(accesskit::Rect {
                x0: f64::from(f32::from(bounds.origin.x) * scale),
                y0: f64::from(f32::from(bounds.origin.y) * scale),
                x1: f64::from(f32::from(bounds.origin.x + bounds.size.width) * scale),
                y1: f64::from(f32::from(bounds.origin.y + bounds.size.height) * scale),
            });
        }
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

    /// Point the focused element's node at the list and its highlighted option.
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
        let (list, options) = popup(Some(1)).nodes(&IDS, 1.0);
        assert_eq!(list.role(), accesskit::Role::ListBox);
        assert_eq!(list.label(), Some("Commands"));
        assert_eq!(list.children(), IDS);
        assert_eq!(list.bounds(), None);
        assert_eq!(options[0].label(), Some("Save"));
        assert_eq!(options[0].description(), Some("File"));
        assert_eq!(options[0].keyboard_shortcut(), Some("Ctrl+S"));
        assert_eq!(options[0].position_in_set(), Some(1));
        assert_eq!(options[1].size_of_set(), Some(2));
        assert_eq!(options[0].is_selected(), Some(false));
        assert_eq!(options[1].is_selected(), Some(true));
    }

    #[test]
    fn the_list_is_placed_in_physical_pixels() {
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::px(10.), gpui::px(20.)),
            gpui::size(gpui::px(100.), gpui::px(50.)),
        );
        let (list, _) = popup(None).with_bounds(Some(bounds)).nodes(&IDS, 2.0);
        let rect = list.bounds().unwrap();
        assert_eq!((rect.x0, rect.y0, rect.x1, rect.y1), (20., 40., 220., 140.));
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
        let (_, options) = popup(Some(5)).nodes(&IDS, 1.0);
        assert!(
            options
                .iter()
                .all(|option| option.is_selected() == Some(false))
        );
    }
}
