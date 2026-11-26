//! Chips for the folders mapped to a label or project (its paths), shown in
//! the header above the issue list: clicking one shows it in the file
//! manager, × removes it, and a last chip adds one.

use gpui::{Action, AnyElement, IntoElement, div, prelude::*, px};
use solilog_core::MappedPath;

use crate::ui::theme::Theme;

/// One chip per path, then the add button. `remove` makes the action
/// removing a path (given as stored); `add` asks for a folder to add.
pub(super) fn path_chips(
    id: &'static str,
    paths: &[MappedPath],
    remove: impl Fn(String) -> Box<dyn Action>,
    add: Box<dyn Action>,
    t: &Theme,
) -> Vec<AnyElement> {
    let mut chips: Vec<AnyElement> = paths
        .iter()
        .enumerate()
        .map(|(ix, mapped)| {
            let remove = remove(mapped.path.clone());
            let text = if mapped.exists {
                mapped.path.clone()
            } else {
                format!("{} (missing)", mapped.path)
            };
            let exists = mapped.exists;
            let full = mapped.absolute.clone();
            let hover_text = t.colors.text;
            div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .pl_2()
                .pr_1()
                .when(t.is_bevel(), |s| s.bg(t.colors.surface))
                .when(!t.is_bevel(), |s| {
                    s.rounded_full().border_1().border_color(t.colors.border)
                })
                .line_height(px(18.))
                .text_color(t.colors.text_muted)
                .child(
                    div()
                        .id((id, ix * 2))
                        .when(exists, |s| {
                            s.cursor_pointer()
                                .hover(move |s| s.text_color(hover_text))
                                .on_click(move |_, _, cx| cx.reveal_path(&full))
                        })
                        .child(text),
                )
                .child(
                    div()
                        .id((id, ix * 2 + 1))
                        .px_1()
                        .when(!t.is_bevel(), |s| s.rounded_full())
                        .cursor_pointer()
                        .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(remove.boxed_clone(), cx);
                        })
                        .child("×"),
                )
                .children(t.raised_bevel_thin())
                .into_any_element()
        })
        .collect();
    chips.push(
        div()
            .id((id, usize::MAX))
            .relative()
            .flex_none()
            .px_2()
            .when(t.is_bevel(), |s| s.bg(t.colors.surface))
            .when(!t.is_bevel(), |s| {
                s.rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(t.colors.border)
            })
            .line_height(px(18.))
            .text_color(t.colors.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.colors.hover).text_color(t.colors.text))
            .child("+ Path")
            .children(t.raised_bevel_thin())
            .on_click(move |_, window, cx| window.dispatch_action(add.boxed_clone(), cx))
            .into_any_element(),
    );
    chips
}
