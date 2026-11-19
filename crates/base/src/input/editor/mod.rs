use gpui::{App, Div, Entity, InteractiveElement as _, IntoElement, RenderOnce, Stateful, Window};

use super::{EditorMode, InputBaseState, InputModeKind};

/// State for source-code editing.
///
/// This is the shared editing engine in its code-editor kind. Languages, line
/// numbers, folding, indent guides, diagnostics, decorations, and the LSP
/// providers exist on this kind only, so an ordinary input or textarea never
/// exposes them.
pub type EditorState = InputBaseState<EditorMode>;

impl InputModeKind for EditorMode {
    const MULTI_LINE: bool = true;
    const CODE_EDITOR: bool = true;

    type Extras = super::EditorExtras;

    fn hover_definition_style(
        state: &InputBaseState<Self>,
        _cx: &App,
    ) -> Option<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        state.hover_definition_style()
    }

    fn hover_definition_hitbox(
        state: &InputBaseState<Self>,
        window: &mut Window,
        _cx: &App,
    ) -> Option<gpui::Hitbox> {
        state.hover_definition_hitbox(window)
    }

    fn reset_language_features(state: &mut InputBaseState<Self>) {
        state.extras.lsp.reset();
    }

    fn reset_annotations(state: &mut InputBaseState<Self>) {
        state.extras.hover_popover = None;
        state.extras.decorations.clear();
        state.extras.range_decorations.clear();
        state.extras.gutter_decorations.clear();
        state.extras.overview_ruler_decorations.clear();
        state.extras.inlays.clear();
    }

    fn editing_syntax_context(state: &InputBaseState<Self>, offset: usize) -> super::SyntaxContext {
        state.syntax_context_at(offset)
    }

    fn adjust_annotations(
        state: &mut InputBaseState<Self>,
        range: &std::ops::Range<usize>,
        new_len: usize,
    ) {
        state.extras.decorations.adjust_for_edit(range, new_len);
        state
            .extras
            .range_decorations
            .adjust_for_edit(range, new_len);
        state
            .extras
            .gutter_decorations
            .adjust_for_edit(range, new_len);
        state
            .extras
            .overview_ruler_decorations
            .adjust_for_edit(range, new_len);
        state.extras.inlays.adjust_for_edit(range, new_len);
        if !state.extras.unnumbered_starts.is_empty() {
            use crate::input::RopeExt as _;
            let text = &state.text;
            let extras = &mut state.extras;
            extras.unnumbered_starts.retain_mut(|start| {
                // A line whose start, or the line break before it, was
                // deleted joins the line before it.
                if *start > range.start && *start <= range.end {
                    return false;
                }
                *start =
                    super::decorations::adjust_following_point_for_edit(*start, range, new_len);
                true
            });
            extras.unnumbered_lines = extras
                .unnumbered_starts
                .iter()
                .map(|&start| text.offset_to_point(start.min(text.len())).row)
                .collect();
            extras.unnumbered_lines.dedup();
        }
    }

    fn refresh_language_features(
        state: &mut InputBaseState<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let text = state.text().clone();
        state.extras.lsp.update(&text, window, cx);
    }

    fn accept_inline_completion(
        state: &mut InputBaseState<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.accept_inline_completion(window, cx)
    }

    fn has_inline_completion(state: &InputBaseState<Self>) -> bool {
        state.has_inline_completion()
    }

    fn on_click(
        state: &mut InputBaseState<Self>,
        event: &gpui::MouseDownEvent,
        offset: usize,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.handle_click_hover_definition(event, offset, window, cx)
    }

    fn clear_hover_state(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.clear_hover_state(cx);
    }

    fn on_text_typed(
        state: &mut InputBaseState<Self>,
        range: &std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_completion_trigger(range, text, window, cx);
    }

    fn clear_inline_completion(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.clear_inline_completion(cx);
    }

    fn hide_context_menu(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.hide_context_menu(cx);
    }

    fn is_context_menu_open(state: &InputBaseState<Self>, cx: &App) -> bool {
        state.is_context_menu_open(cx)
    }

    fn handle_context_menu_action(
        state: &mut InputBaseState<Self>,
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.handle_action_for_context_menu(action, window, cx)
    }

    fn on_hover_definition(
        state: &mut InputBaseState<Self>,
        offset: usize,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_hover_definition(offset, window, cx);
    }

    fn on_mouse_move(
        state: &mut InputBaseState<Self>,
        offset: usize,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_mouse_move(offset, event, window, cx);
    }

    fn drive_highlighter(
        highlighter: &std::rc::Rc<std::cell::RefCell<Option<Box<dyn super::InputHighlighter>>>>,
        edit: super::InputEdit,
        text: &ropey::Rope,
        folding: bool,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let mut highlighter = highlighter.borrow_mut();
        let Some(highlighter) = highlighter.as_mut() else {
            return;
        };
        highlighter.update(Some(edit), text, folding, window, cx);
    }

    fn drive_highlighter_batch(
        highlighter: &std::rc::Rc<std::cell::RefCell<Option<Box<dyn super::InputHighlighter>>>>,
        edits: &[(super::InputEdit, ropey::Rope)],
        folding: bool,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let mut highlighter = highlighter.borrow_mut();
        let Some(highlighter) = highlighter.as_mut() else {
            return;
        };
        highlighter.update_batch(edits, folding, window, cx);
    }

    fn register_actions(
        element: Stateful<Div>,
        entity: &Entity<InputBaseState<Self>>,
        window: &mut Window,
    ) -> Stateful<Div> {
        element
            .key_context("Input mode=editor")
            .on_action(window.listener_for(entity, InputBaseState::on_action_toggle_code_actions))
            .on_action(window.listener_for(entity, InputBaseState::on_action_show_completions))
            .on_action(window.listener_for(entity, InputBaseState::on_action_go_to_definition))
    }
}

impl EditorState {
    /// The LSP providers and their cached results.
    ///
    /// This exists on the editor alone: an ordinary input or textarea has no
    /// language server, and no field to reach one through.
    pub fn lsp(&self) -> &super::Lsp {
        &self.extras.lsp
    }

    /// The LSP providers, mutably. Configure the providers through this.
    pub fn lsp_mut(&mut self) -> &mut super::Lsp {
        &mut self.extras.lsp
    }

    /// Syntax context at `offset`, or `Code` when the language has no provider.
    pub(crate) fn syntax_context_at(&self, offset: usize) -> super::SyntaxContext {
        self.mode.syntax_context_at(&self.text, offset)
    }
}

/// An unstyled source-code editor.
#[derive(IntoElement)]
pub struct Editor {
    state: Entity<EditorState>,
}

impl Editor {
    pub fn new(state: &Entity<EditorState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl RenderOnce for Editor {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.state
    }
}

/// What a code editor exposes to the renderer. See [`crate::input::InputExtras`].
impl crate::input::InputExtras for super::EditorExtras {
    fn decoration_layers(&self) -> Vec<&[super::TextDecoration]> {
        self.decorations.iter().collect()
    }

    fn range_decorations(&self, ranges: &[std::ops::Range<usize>]) -> Vec<&super::RangeDecoration> {
        self.range_decorations.intersecting(ranges)
    }

    fn inlays(&self, ranges: &[std::ops::Range<usize>]) -> Vec<&super::Inlay> {
        self.inlays.intersecting(ranges)
    }

    fn gutter_decorations_width(&self) -> Option<gpui::Pixels> {
        self.gutter_decorations
            .has_collections()
            .then_some(self.gutter_decorations_width)
    }

    fn gutter_margin(&self) -> gpui::Pixels {
        self.gutter_margin
    }

    fn gutter_decorations(&self, range: std::ops::Range<usize>) -> Vec<&super::GutterDecoration> {
        self.gutter_decorations.intersecting(&[range])
    }

    fn line_number_of(&self, buffer_line: usize) -> Option<usize> {
        // `unnumbered_lines` is kept sorted and without repeats, so `before`
        // counts those before the line, and is at most `buffer_line`.
        match self.unnumbered_lines.binary_search(&buffer_line) {
            Ok(_) => None,
            Err(before) => Some(buffer_line + 1 - before),
        }
    }

    fn overview_ruler_decorations(&self) -> Vec<&super::OverviewRulerDecoration> {
        self.overview_ruler_decorations.iter().flatten().collect()
    }

    fn overview_ruler_revision(&self) -> u64 {
        self.overview_ruler_decorations.revision()
    }

    fn semantic_token_styles(
        &self,
        text: &ropey::Rope,
        range: &std::ops::Range<usize>,
        resolver: &dyn crate::input::HighlightStyleResolver,
    ) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        self.lsp.semantic_tokens_for_range(text, range, resolver)
    }

    fn document_color_swatches(
        &self,
        text: &ropey::Rope,
        range: &std::ops::Range<usize>,
    ) -> Vec<(std::ops::Range<usize>, gpui::Hsla)> {
        self.lsp.document_colors_for_range(text, range)
    }

    fn hover_symbol_range(&self) -> Option<std::ops::Range<usize>> {
        self.hover_popover
            .as_ref()
            .map(|session| session.symbol_range.clone())
    }

    fn inline_completion_item(&self) -> Option<&lsp_types::InlineCompletionItem> {
        self.inline_completion.item.as_ref()
    }

    fn context_menu_capabilities(&self) -> (bool, bool) {
        (
            self.lsp.definition_provider.is_some(),
            !self.lsp.code_action_providers.is_empty(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::input::InputExtras as _;

    #[test]
    fn unnumbered_lines_have_no_number_and_the_rest_count_without_them() {
        let extras = super::super::EditorExtras {
            unnumbered_lines: vec![1, 2, 5],
            ..Default::default()
        };
        let numbers: Vec<Option<usize>> = (0..7).map(|line| extras.line_number_of(line)).collect();
        assert_eq!(
            numbers,
            [Some(1), None, None, Some(2), Some(3), None, Some(4)]
        );
        let plain = super::super::EditorExtras::default();
        assert_eq!(plain.line_number_of(4), Some(5));
    }
}
