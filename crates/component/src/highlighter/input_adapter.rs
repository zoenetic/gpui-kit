use std::{
    cell::RefCell,
    ops::Range,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui::{HighlightStyle, SharedString, Task};
use gpui_base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit as BaseInputEdit, InputHighlighter,
    InputHighlighterFactory,
};
use ropey::Rope;
use tree_sitter::{InputEdit, ParseOptions, Point};

use super::{LanguageRegistry, SyntaxHighlighter, highlighter::parse_input_bytes};

pub(crate) fn input_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|language| {
        LanguageRegistry::singleton().has_parser(language).then(|| {
            Box::new(TreeSitterInputHighlighter::new(language)) as Box<dyn InputHighlighter>
        })
    })
}

const SYNC_PARSE_TIMEOUT: Duration = Duration::from_millis(2);
const SYNC_PARSE_MAX_BYTES: usize = 256 * 1024;
const PARSE_DEBOUNCE: Duration = Duration::from_millis(150);

struct TreeSitterInputHighlighter {
    inner: Rc<RefCell<SyntaxHighlighter>>,
    parse_task: Rc<RefCell<Option<Task<()>>>>,
}

impl TreeSitterInputHighlighter {
    fn new(language: &str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(SyntaxHighlighter::new(language))),
            parse_task: Rc::new(RefCell::new(None)),
        }
    }

    /// Settle after the edits reached the tree: drop a pending background
    /// parse when the synchronous one `completed`, or schedule one for `text`.
    fn finish_update(
        &mut self,
        completed: bool,
        text: &Rope,
        folding: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<EditorState>,
    ) {
        if completed {
            self.parse_task.borrow_mut().take();
            return;
        }

        let highlighter = self.inner.clone();
        let parse_task = self.parse_task.clone();
        let language = highlighter.borrow().language().clone();
        let old_tree = highlighter.borrow().tree().cloned();
        let injection_data = highlighter.borrow().injection_parse_data();
        let text = text.clone();
        let text_for_apply = text.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        let task = cx.spawn_in(window, async move |entity, cx| {
            struct CancelOnDrop(Arc<AtomicBool>);
            impl Drop for CancelOnDrop {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Relaxed);
                }
            }
            let _cancel_guard = CancelOnDrop(cancel.clone());
            cx.background_executor().timer(PARSE_DEBOUNCE).await;

            let parse_cancel = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (mut parser, grammar) =
                        LanguageRegistry::singleton().parser(&language).ok()?;
                    parser.set_language(&grammar).ok()?;
                    let mut progress = |_: &tree_sitter::ParseState| {
                        if parse_cancel.load(Ordering::Relaxed) {
                            std::ops::ControlFlow::Break(())
                        } else {
                            std::ops::ControlFlow::Continue(())
                        }
                    };
                    let options = ParseOptions::new().progress_callback(&mut progress);
                    let tree = parser.parse_with_options(
                        &mut |offset, _| parse_input_bytes(&text, offset),
                        old_tree.as_ref(),
                        Some(options),
                    )?;
                    if parse_cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    let injections = injection_data.map_or_else(Default::default, |data| {
                        SyntaxHighlighter::compute_injection_layers(data, &tree, &text)
                    });
                    let folds = if folding {
                        extract_fold_ranges(&tree)
                    } else {
                        Vec::new()
                    };
                    Some((tree, injections, folds))
                })
                .await;

            if let Some((tree, injections, folds)) = result {
                highlighter
                    .borrow_mut()
                    .apply_background_tree(tree, &text_for_apply, injections);
                let _ = entity.update(cx, |state, cx| {
                    state.apply_highlighter_fold_candidates(folds, cx);
                });
            }
        });
        parse_task.borrow_mut().replace(task);
    }
}

impl SyntaxHighlighter {
    pub(crate) fn update_input(
        &mut self,
        edit: Option<BaseInputEdit>,
        text: &Rope,
        timeout: Option<Duration>,
    ) -> bool {
        self.update(edit.map(to_tree_sitter_edit), text, timeout)
    }
}

impl InputHighlighter for TreeSitterInputHighlighter {
    fn language(&self) -> SharedString {
        self.inner.borrow().language().clone()
    }

    fn update(
        &mut self,
        edit: Option<BaseInputEdit>,
        text: &Rope,
        folding: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<EditorState>,
    ) {
        let edit = edit.map(to_tree_sitter_edit);
        let completed = {
            let mut highlighter = self.inner.borrow_mut();
            if text.len() > SYNC_PARSE_MAX_BYTES {
                highlighter.edit_tree(edit, text);
                false
            } else {
                highlighter.update(edit, text, Some(SYNC_PARSE_TIMEOUT))
            }
        };
        self.finish_update(completed, text, folding, window, cx);
    }

    fn update_batch(
        &mut self,
        edits: &[(BaseInputEdit, Rope)],
        folding: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<EditorState>,
    ) {
        let Some((_, text)) = edits.last() else {
            return;
        };
        let edits: Vec<InputEdit> = edits
            .iter()
            .map(|(edit, _)| to_tree_sitter_edit(*edit))
            .collect();
        let completed = {
            let mut highlighter = self.inner.borrow_mut();
            if text.len() > SYNC_PARSE_MAX_BYTES {
                for edit in edits {
                    highlighter.edit_tree(Some(edit), text);
                }
                false
            } else {
                highlighter.update_edits(&edits, text, Some(SYNC_PARSE_TIMEOUT))
            }
        };
        self.finish_update(completed, text, folding, window, cx);
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        self.inner.borrow().styles(range, resolver)
    }
    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        self.inner
            .borrow()
            .tree()
            .map(extract_fold_ranges)
            .unwrap_or_default()
    }

    fn fold_ranges_for_edit(&self, range: Range<usize>, _: &Rope) -> Vec<FoldRange> {
        self.inner
            .borrow()
            .tree()
            .map(|tree| extract_fold_ranges_in_range(tree, range))
            .unwrap_or_default()
    }
}

fn to_tree_sitter_edit(edit: BaseInputEdit) -> InputEdit {
    InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: Point::new(edit.start_position.row, edit.start_position.column),
        old_end_position: Point::new(edit.old_end_position.row, edit.old_end_position.column),
        new_end_position: Point::new(edit.new_end_position.row, edit.new_end_position.column),
    }
}

/// The line a fold over `node` ends on, which stays visible.
///
/// A node that closes with a bracket keeps that line in view, so a folded
/// block reads `fn f() {` … `}`. One that doesn't, such as a list of imports
/// or a block that ends with its last statement, folds through its own last
/// line, and the line after it stays in view instead.
fn fold_end(node: tree_sitter::Node, last_row: usize) -> usize {
    let end = node.end_position();
    // The node's last token.
    let mut last = node;
    while let Some(child) = last.child(last.child_count().saturating_sub(1) as u32) {
        last = child;
    }
    let closed = end.column == 0 || matches!(last.kind(), "}" | ")" | "]" | ">");
    if closed || end.row >= last_row {
        end.row
    } else {
        end.row + 1
    }
}

fn extract_fold_ranges(tree: &tree_sitter::Tree) -> Vec<FoldRange> {
    extract_fold_ranges_in_range(tree, 0..usize::MAX)
}

fn extract_fold_ranges_in_range(
    tree: &tree_sitter::Tree,
    byte_range: Range<usize>,
) -> Vec<FoldRange> {
    fn collect(
        node: tree_sitter::Node,
        bytes: &Range<usize>,
        last_row: usize,
        ranges: &mut Vec<FoldRange>,
    ) {
        if node.end_byte() <= bytes.start || node.start_byte() >= bytes.end {
            return;
        }
        let start = node.start_position().row;
        let end = fold_end(node, last_row);
        if end.saturating_sub(start) < 2 {
            return;
        }
        ranges.push(FoldRange::new(start, end));
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, bytes, last_row, ranges);
        }
    }

    let root = tree.root_node();
    let last_row = root.end_position().row;
    let mut ranges = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        collect(child, &byte_range, last_row, &mut ranges);
    }
    ranges.sort_by_key(|range| range.start_line);
    ranges.dedup_by_key(|range| range.start_line);
    ranges
}

#[cfg(all(test, feature = "tree-sitter-kotlin", feature = "tree-sitter-rust"))]
mod tests {
    use super::extract_fold_ranges;
    use gpui_base::input::FoldRange;

    fn folds(language: tree_sitter::Language, text: &str) -> Vec<FoldRange> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        extract_fold_ranges(&parser.parse(text, None).unwrap())
    }

    /// A block closed by a bracket keeps the bracket's line in view; a list
    /// of imports, which has none, folds through its last import.
    #[test]
    fn test_fold_ranges_end_after_unbracketed_nodes() {
        let kotlin = "package a\n\nimport a.B\nimport a.C\nimport a.D\n\nclass A\n";
        let ranges = folds(tree_sitter_kotlin_sg::LANGUAGE.into(), kotlin);
        assert!(ranges.contains(&FoldRange::new(2, 5)), "{ranges:?}");

        let rust = "fn a() {\n    1;\n    2;\n}\n";
        let ranges = folds(tree_sitter_rust::LANGUAGE.into(), rust);
        assert!(ranges.contains(&FoldRange::new(0, 3)), "{ranges:?}");
    }
}
