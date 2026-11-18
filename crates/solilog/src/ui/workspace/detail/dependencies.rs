//! The open issue's dependencies: the issues it depends on and those that
//! depend on it. Clicking one opens it; adding and removing go through
//! `issue::AddDependency` / `issue::RemoveDependency`.

use gpui::{
    Action, ClickEvent, FocusHandle, FontWeight, IntoElement, SharedString, div, prelude::*, px,
    rgb,
};
use solilog_core::{IssueId, Status};

use super::IssueDetail;
use crate::ui::actions::issue::{AddDependency, Open, RemoveDependency};
use crate::ui::status_icon::status_icon;
use crate::ui::theme;
use crate::ui::workspace::Workspace;

impl Workspace {
    pub(super) fn render_dependencies(&self, detail: &IssueDetail) -> impl IntoElement + use<> {
        let focus = detail.focus.clone();
        let add = div()
            .id("dependency-add")
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(rgb(theme::TEXT_MUTED))
            .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
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
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(title),
                    )
                    .children(ids.iter().map(|&id| {
                        let (title, status) = self
                            .issue_summary(id)
                            .unwrap_or_else(|| (String::new(), Status::Todo));
                        dependency_row(id, title, status, removable, &focus)
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
    }
}

fn dependency_row(
    id: IssueId,
    title: String,
    status: Status,
    removable: bool,
    focus: &FocusHandle,
) -> impl IntoElement + use<> {
    let group: SharedString = format!("dependency-{}", id.0).into();
    let remove = removable.then(|| {
        div()
            .id(("dependency-remove", id.0 as u64))
            .flex_none()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .text_color(rgb(theme::TEXT_MUTED))
            .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
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
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::HOVER)))
        .on_click({
            let focus = focus.clone();
            move |_: &ClickEvent, window, cx| {
                window.focus(&focus);
                window.dispatch_action(Open { id: Some(id) }.boxed_clone(), cx);
            }
        })
        .child(status_icon(status))
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(id.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(status.is_closed(), |s| s.text_color(rgb(theme::TEXT_MUTED)))
                .child(title),
        )
        .children(remove)
}
