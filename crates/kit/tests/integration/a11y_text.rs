use crate::common;
use gpui_kit::{
    AppContext, Context, TestAppContext, Window, component::label::Label, div, prelude::*,
};

/// The Quick Start renders its greeting with `Label`, which exposes its text in
/// the accessibility tree. GPUI only adds elements with an ID to the tree, so a
/// bare string child stays invisible to screen readers.
struct Greeting;
impl Render for Greeting {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(Label::new("Hello, World!"))
            .child("Bare string")
    }
}

#[gpui_kit::test]
fn quick_start_text_is_in_the_accessibility_tree(cx: &mut TestAppContext) {
    // GPUI Fast's test window builds accessibility data only when requested.
    // Re-run in a child so the cached flag is set before any frame, without
    // changing the process environment during parallel tests.
    if std::env::var("GPUI_A11Y_ACTIVE").as_deref() != Ok("1") {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a11y_text::quick_start_text_is_in_the_accessibility_tree",
                "--nocapture",
            ])
            .env("GPUI_A11Y_ACTIVE", "1")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "accessibility tree check failed: {status}"
        );
        return;
    }

    cx.update(gpui_kit::init);
    let (handle, _) = common::open_window(cx, None, |_, cx| cx.new(|_| Greeting));
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        let tree = window.debug_a11y_tree_json().expect("accessibility tree");
        assert!(tree.contains("\"Hello, World!\""), "{tree}");
        assert!(!tree.contains("Bare string"), "{tree}");
    })
    .unwrap();
}

#[cfg(feature = "component")]
/// An editor showing a list while it keeps focus, as a completion list or palette.
struct Completing {
    editor: gpui_kit::Entity<gpui_kit::component::input::EditorState>,
}
#[cfg(feature = "component")]
impl Render for Completing {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(gpui_kit::component::input::Editor::new(&self.editor).aria_label("Code"))
    }
}

#[cfg(feature = "component")]
#[gpui_kit::test]
fn an_editors_popup_list_is_its_child_and_names_the_active_option(cx: &mut TestAppContext) {
    use gpui_kit::base::input::{InputAccessibilityOption, InputAccessibilityPopup};
    // As above: the tree is built only when asked for before any frame.
    if std::env::var("GPUI_A11Y_ACTIVE").as_deref() != Ok("1") {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a11y_text::an_editors_popup_list_is_its_child_and_names_the_active_option",
                "--nocapture",
            ])
            .env("GPUI_A11Y_ACTIVE", "1")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "accessibility tree check failed: {status}"
        );
        return;
    }

    cx.update(gpui_kit::init);
    let (handle, view) = common::open_window(cx, None, |window, cx| {
        let editor = cx
            .new(|cx| gpui_kit::component::input::EditorState::new(window, cx).default_value("ab"));
        cx.new(|_| Completing { editor })
    });
    let popup = InputAccessibilityPopup::new()
        .with_label(Some("Completions".into()))
        .with_options(vec![
            InputAccessibilityOption::new("abs"),
            InputAccessibilityOption::new("about"),
        ])
        .with_active(Some(1));
    cx.update_window(handle.into(), |_, window, cx| {
        let editor = view.read(cx).editor.clone();
        editor.update(cx, |editor, cx| {
            editor.focus(window, cx);
            editor.set_accessibility_popup(Some(popup), cx);
        });
        window.draw(cx).clear(cx);
        let tree = window.debug_a11y_tree_json().expect("accessibility tree");
        let tree: serde_json::Value = serde_json::from_str(&tree).unwrap();
        let nodes = tree["nodes"].as_object().unwrap();
        let find = |role: &str| {
            let found: Vec<_> = nodes
                .iter()
                .filter(|(_, node)| node["aria"]["role"] == role)
                .collect();
            assert_eq!(found.len(), 1, "one {role}: {tree:#}");
            (serde_json::Value::from(found[0].0.clone()), found[0].1)
        };
        let (_, editor) = find("MultilineTextInput");
        let (list_id, list) = find("ListBox");
        let children = |node: &serde_json::Value| node["children"].as_array().unwrap().clone();
        // The list is the editor's child; its options are the list's, not the editor's.
        assert!(children(editor).contains(&list_id), "{tree:#}");
        assert_eq!(children(list).len(), 2, "{tree:#}");
        for option in children(list) {
            assert!(!children(editor).contains(&option), "{tree:#}");
        }
        assert_eq!(list["aria"]["label"], "Completions");
        assert_eq!(editor["aria"]["expanded"], true);
        let selected: Vec<_> = nodes
            .values()
            .filter(|node| node["aria"]["role"] == "ListBoxOption")
            .filter(|node| node["aria"]["selected"] == true)
            .map(|node| node["aria"]["label"].clone())
            .collect();
        assert_eq!(selected, ["about"]);

        // Cleared, the list goes.
        let editor = view.read(cx).editor.clone();
        editor.update(cx, |editor, cx| editor.set_accessibility_popup(None, cx));
        window.draw(cx).clear(cx);
        let tree = window.debug_a11y_tree_json().expect("accessibility tree");
        assert!(!tree.contains("ListBox"), "{tree}");
    })
    .unwrap();
}
