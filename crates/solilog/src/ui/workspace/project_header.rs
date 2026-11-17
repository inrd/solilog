//! The header above the issue list while it shows a project: its name,
//! status, progress (closed issues out of all of them) and default labels.
//!
//! Key context: `ProjectHeader`, outside `IssueList`, so typing in its
//! fields doesn't trigger the list's single-key bindings.

use gpui::{
    Action, ClickEvent, Context, FontWeight, IntoElement, div, prelude::*, px, relative, rgb,
};
use solilog_core::Project;

use super::Workspace;
use super::scope::Source;
use crate::ui::actions::project;
use crate::ui::status_icon::project_status_icon;
use crate::ui::theme;

pub const CONTEXT: &str = "ProjectHeader";

impl Workspace {
    /// The project the issue list shows, if any.
    pub(super) fn shown_project(&self) -> Option<&Project> {
        match &self.scope.source {
            Source::Project(name) => self.projects.iter().find(|p| p.name == *name),
            _ => None,
        }
    }

    pub(super) fn render_project_header(
        &self,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let status = project.status;
        let status_chip = div()
            .id("project-status")
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded_full()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_size(px(12.))
            .line_height(px(20.))
            .text_color(rgb(theme::TEXT_MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
            .child(project_status_icon(status))
            .child(status.label())
            .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                window.dispatch_action(project::SetStatus::default().boxed_clone(), cx);
            }));

        div()
            .key_context(CONTEXT)
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_3()
            .pb_2()
            .border_b_1()
            .border_color(rgb(theme::DIVIDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(18.))
                            .line_height(px(26.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(project.name.clone()),
                    )
                    .child(status_chip)
                    .child(progress(project)),
            )
            .child(self.default_labels(project, cx))
    }

    /// The project's default labels, each removable, and a button adding one.
    fn default_labels(
        &self,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let chips = project.labels.iter().enumerate().map(|(ix, label)| {
            let remove = project::RemoveLabel {
                name: None,
                label: Some(label.clone()),
            };
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .pl_2()
                .pr_1()
                .rounded_full()
                .border_1()
                .border_color(rgb(theme::BORDER))
                .line_height(px(18.))
                .text_color(rgb(self.label_chip_color(label)))
                .child(label.clone())
                .child(
                    div()
                        .id(("project-label-remove", ix))
                        .px_1()
                        .rounded_full()
                        .cursor_pointer()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
                        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                            window.dispatch_action(remove.boxed_clone(), cx);
                        }))
                        .child("×"),
                )
        });
        let add = div()
            .id("project-label-add")
            .flex_none()
            .px_2()
            .rounded_full()
            .border_1()
            .border_dashed()
            .border_color(rgb(theme::BORDER))
            .line_height(px(18.))
            .text_color(rgb(theme::TEXT_MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
            .child(if project.labels.is_empty() {
                "+ Default label"
            } else {
                "+"
            })
            .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                window.dispatch_action(project::AddLabel::default().boxed_clone(), cx);
            }));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .text_size(px(12.))
            .children(chips)
            .child(add)
    }
}

/// Closed issues out of all of the project's, with a thin bar.
fn progress(project: &Project) -> impl IntoElement + use<> {
    let (closed, total) = (project.closed_issue_count, project.issue_count);
    let row = div()
        .ml_auto()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(rgb(theme::TEXT_MUTED));
    if total == 0 {
        return row.child("No issues yet");
    }
    let complete = closed == total;
    row.child(
        div()
            .when(complete, |s| s.text_color(rgb(theme::ACCENT)))
            .child(format!("{closed}/{total} closed")),
    )
    .child(
        div()
            .w(px(120.))
            .h(px(4.))
            .rounded_full()
            .bg(rgb(theme::HIGHLIGHT))
            .child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(rgb(theme::ACCENT))
                    .w(relative(closed as f32 / total as f32)),
            ),
    )
}
