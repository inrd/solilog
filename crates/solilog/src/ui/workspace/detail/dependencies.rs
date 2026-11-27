//! The open issue's dependencies: the issues it depends on and those that
//! depend on it, then the issues that mention it. Clicking one opens it; adding and removing go through
//! `issue::AddDependency` / `issue::RemoveDependency`.

use gpui::{
    Action, ClickEvent, FocusHandle, FontWeight, IntoElement, Pixels, SharedString, div,
    prelude::*, px,
};
use solilog_core::{IssueId, Status};

use super::IssueDetail;
use crate::ui::actions::issue::{AddDependency, Open, RemoveDependency};
use crate::ui::status_icon::status_icon;
use crate::ui::theme;
use crate::ui::tooltip::truncated_text;
use crate::ui::workspace::Workspace;

impl Workspace {
    pub(super) fn render_dependencies(
        &self,
        detail: &IssueDetail,
        t: &theme::Theme,
    ) -> impl IntoElement + use<> {
        let focus = detail.focus.clone();
        let add = div()
            .id("dependency-add")
            .px_1()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(t.colors.text_muted)
            .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
            .child("Add…")
            .on_click({
                let focus = focus.clone();
                move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(AddDependency::default().boxed_clone(), cx);
                }
            });
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Dependencies"),
            )
            .child(add);

        let issue = detail.issue();
        let group = |title: &'static str, ids: &[IssueId], removable: bool| {
            (!ids.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_1()
                            .text_size(px(12.))
                            .text_color(t.colors.text_muted)
                            .child(title),
                    )
                    .children(ids.iter().map(|&id| {
                        let (title, status) = self
                            .issue_summary(id)
                            .unwrap_or_else(|| (String::new(), Status::Todo));
                        dependency_row(id, title, status, removable, self.id_width, &focus, t)
                    }))
            })
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(header)
            .children(group("Depends on", &issue.depends_on, true))
            .children(group("Needed by", &issue.dependents, false))
            .children(group("Mentioned in", detail.mentioned_by(), false))
    }
}

fn dependency_row(
    id: IssueId,
    title: String,
    status: Status,
    removable: bool,
    id_width: Pixels,
    focus: &FocusHandle,
    t: &theme::Theme,
) -> impl IntoElement + use<> {
    let group: SharedString = format!("dependency-{}", id.0).into();
    let remove = removable.then(|| {
        div()
            .id(("dependency-remove", id.0 as u64))
            .flex_none()
            .px_1()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .text_color(t.colors.text_muted)
            .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
            .child("×")
            .on_click({
                let focus = focus.clone();
                move |_: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    window.focus(&focus);
                    let action = RemoveDependency { on: Some(id) };
                    window.dispatch_action(action.boxed_clone(), cx);
                }
            })
    });
    div()
        .id(("dependency", id.0 as u64))
        .group(group)
        .flex()
        .items_center()
        .gap_2()
        .px_1()
        .py(px(2.))
        .rounded(t.radius_sm())
        .cursor_pointer()
        .hover(|s| s.bg(t.colors.hover))
        .on_click({
            let focus = focus.clone();
            move |_: &ClickEvent, window, cx| {
                window.focus(&focus);
                window.dispatch_action(Open { id: Some(id) }.boxed_clone(), cx);
            }
        })
        .child(status_icon(status, t))
        .child(
            div()
                .w(id_width)
                .flex_none()
                .whitespace_nowrap()
                .text_color(t.colors.text_muted)
                .child(id.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(status.is_closed(), |s| s.text_color(t.colors.text_muted))
                .child(truncated_text(("dependency-title", id.0 as u64), title)),
        )
        .children(remove)
}
