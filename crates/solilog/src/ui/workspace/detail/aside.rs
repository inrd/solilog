//! The open issue's side pane: the short facts about it (status, when and
//! by whom it was created and last changed, archived), its projects and
//! labels, links, attachments and dependencies, leaving the main column to
//! the description, checklist and notes. It scrolls on its own. When the
//! window is too narrow for two columns, its sections go into the main
//! column instead, after the description (see `render_detail`).

use gpui::{
    Action, AnyElement, ClickEvent, Context, FocusHandle, IntoElement, div, prelude::*, px,
};
use solilog_core::{Actor, format_age};

use super::super::section::{section_action, section_count, section_header};
use super::super::{Workspace, chip, project_chip};
use super::IssueDetail;
use crate::ui::actions::issue::{
    AddLabel, AddToProject, RemoveFromProject, RemoveLabel, SetStatus,
};
use crate::ui::status_icon::status_icon;
use crate::ui::theme;

/// Width of the side pane.
pub(super) const WIDTH: f32 = 280.;

/// The narrowest the main column gets beside the pane; narrower, the pane's
/// sections go into the main column.
pub(super) const MIN_MAIN_WIDTH: f32 = 480.;

impl Workspace {
    /// The pane's sections, in order (the caller sets them apart).
    pub(super) fn render_aside_sections(
        &self,
        detail: &IssueDetail,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = theme::current(cx);
        vec![
            self.render_facts(detail, &t).into_any_element(),
            self.render_projects(detail, &t).into_any_element(),
            self.render_labels(detail, &t).into_any_element(),
            self.render_links(detail, &t).into_any_element(),
            self.render_attachments(detail, &t).into_any_element(),
            self.render_dependencies(detail, &t).into_any_element(),
        ]
    }

    /// Status (clicking it opens the status picker), creation, last change,
    /// and whether the issue is archived.
    fn render_facts(&self, detail: &IssueDetail, t: &theme::Theme) -> impl IntoElement + use<> {
        let issue = detail.issue();
        let focus = detail.focus.clone();
        let row = |name: &'static str, value: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .w(px(64.))
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(name),
                )
                .child(div().flex_1().min_w_0().child(value))
        };
        // The author below the time: agent names are long.
        let when = |time: i64, by: &Actor| {
            div()
                .flex()
                .flex_col()
                .child(format_age(time))
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(t.colors.text_muted)
                        .child(format!("by {by}")),
                )
                .into_any_element()
        };
        let status = div()
            .id("aside-status")
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .ml(px(-4.))
            .rounded(t.radius_sm())
            .cursor_pointer()
            .hover(|s| s.bg(t.colors.hover))
            .child(status_icon(issue.status, t))
            .child(issue.status.label())
            .on_click(move |_: &ClickEvent, window, cx| {
                window.focus(&focus);
                window.dispatch_action(SetStatus::default().boxed_clone(), cx);
            });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .line_height(px(24.))
            .child(row("Status", div().flex().child(status).into_any_element()))
            .child(row("Created", when(issue.created_at, &issue.created_by)))
            .child(row("Updated", when(issue.updated_at, &issue.updated_by)))
            .when(issue.archived, |s| {
                s.child(row(
                    "Archived",
                    div()
                        .flex()
                        .child(chip("archived".into(), t.colors.text_muted, t))
                        .into_any_element(),
                ))
            })
    }

    fn render_projects(&self, detail: &IssueDetail, t: &theme::Theme) -> impl IntoElement + use<> {
        let projects = &detail.issue().projects;
        let chips = projects
            .iter()
            .map(|p| project_chip(p.clone(), t).into_any_element())
            .collect();
        chips_section(
            "Projects",
            "aside-projects",
            chips,
            AddToProject::default().boxed_clone(),
            RemoveFromProject::default().boxed_clone(),
            "In no project",
            &detail.focus,
            t,
        )
    }

    fn render_labels(&self, detail: &IssueDetail, t: &theme::Theme) -> impl IntoElement + use<> {
        let labels = &detail.issue().labels;
        let chips = labels
            .iter()
            .map(|l| chip(l.clone(), self.label_chip_color(l, t), t).into_any_element())
            .collect();
        chips_section(
            "Labels",
            "aside-labels",
            chips,
            AddLabel::default().boxed_clone(),
            RemoveLabel::default().boxed_clone(),
            "No labels",
            &detail.focus,
            t,
        )
    }
}

/// A section of chips, with "+ Add" and, when there are chips, "Remove"
/// (each opening its picker).
#[allow(clippy::too_many_arguments)]
fn chips_section(
    title: &'static str,
    id: &'static str,
    chips: Vec<AnyElement>,
    add: Box<dyn Action>,
    remove: Box<dyn Action>,
    empty: &'static str,
    focus: &FocusHandle,
    t: &theme::Theme,
) -> impl IntoElement + use<> {
    let count = chips.len();
    let mut actions = Vec::new();
    if count > 0 {
        actions.push(
            section_action((id, 0u64), "Remove", remove, Some(focus.clone()), t).into_any_element(),
        );
    }
    actions
        .push(section_action((id, 1u64), "+ Add", add, Some(focus.clone()), t).into_any_element());
    div()
        .flex()
        .flex_col()
        .gap_2()
        .line_height(px(24.))
        .child(section_header(title, section_count(count, t), actions))
        .child(if count == 0 {
            div()
                .px_1()
                .line_height(px(20.))
                .text_size(px(12.))
                .text_color(t.colors.text_muted)
                .child(empty)
        } else {
            div().flex().flex_wrap().gap_1().children(chips)
        })
}
