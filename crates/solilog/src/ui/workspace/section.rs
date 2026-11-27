//! Titled sections (decisions, attachments, dependencies, checklist,
//! notes): the same header row everywhere, and in the open issue a rule
//! above each, so the blocks read apart at a glance.

use gpui::{
    Action, AnyElement, ClickEvent, Div, ElementId, FocusHandle, FontWeight, IntoElement, Stateful,
    div, prelude::*, px,
};

use crate::ui::theme;

/// A section's header row: its title, then `summary` (a count, progress),
/// then `actions` on the right.
pub(super) fn section_header(
    title: &'static str,
    summary: Option<AnyElement>,
    actions: impl IntoIterator<Item = AnyElement>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .children(summary)
        .child(div().flex_1())
        .children(actions)
}

/// A muted count for [`section_header`], or nothing when it is zero.
pub(super) fn section_count(count: usize, t: &theme::Theme) -> Option<AnyElement> {
    (count > 0).then(|| {
        div()
            .flex_none()
            .text_size(px(12.))
            .text_color(t.colors.text_muted)
            .child(count.to_string())
            .into_any_element()
    })
}

/// A small link-like button for [`section_header`]'s actions
/// ("+ Add", "+ Record"); it dispatches `action`, from `focus` if given
/// (so the handlers of that element's context get it).
pub(super) fn section_action(
    id: impl Into<ElementId>,
    label: &'static str,
    action: Box<dyn Action>,
    focus: Option<FocusHandle>,
    t: &theme::Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_1()
        .rounded(t.radius_sm())
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(t.colors.text_muted)
        .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
        .child(label)
        .on_click(move |_: &ClickEvent, window, cx| {
            if let Some(focus) = &focus {
                window.focus(focus);
            }
            window.dispatch_action(action.boxed_clone(), cx);
        })
}

/// Sets a section of the open issue apart from what comes before it: space,
/// then a rule (etched in bevel themes) above its header.
pub(super) fn issue_section(content: impl IntoElement, t: &theme::Theme) -> Div {
    let rule = if t.is_bevel() {
        div()
            .child(div().h(px(1.)).bg(t.colors.bevel_shadow))
            .child(div().h(px(1.)).bg(t.colors.bevel_light))
    } else {
        div().h(px(1.)).bg(t.colors.divider)
    };
    div()
        .mt_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(rule)
        .child(content)
}
