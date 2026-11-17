//! The header above the issue list while it shows a project: its name,
//! status, progress (closed issues out of all of them), default labels and
//! description.
//!
//! The description is edited in place and saved like an open issue's (see
//! `detail.rs`): shortly after typing stops, when it loses focus, and before
//! the list shows something else. A long one is cut to a few lines unless
//! it is being edited.
//!
//! Key context: `ProjectHeader`, outside `IssueList`, so typing in its
//! fields doesn't trigger the list's single-key bindings.

use gpui::{
    Action, ClickEvent, Context, Entity, Focusable, FontWeight, IntoElement, Task, Window, div,
    prelude::*, px, relative, rgb,
};
use solilog_core::{Project, ProjectUpdate};

use super::Workspace;
use super::detail::{AUTOSAVE_DELAY, description_changed};
use super::scope::Source;
use crate::ui::actions::project;
use crate::ui::status_icon::project_status_icon;
use crate::ui::text_input::{Changed, TextInput};
use crate::ui::theme;

pub const CONTEXT: &str = "ProjectHeader";

/// Height of a line of the description.
const LINE_HEIGHT: f32 = 20.;
/// Lines of a long description shown while it isn't being edited.
const COLLAPSED_LINES: usize = 3;

/// The description field of the project header. One for the workspace: it
/// holds the description of whichever project is shown.
pub(super) struct ProjectDescription {
    input: Entity<TextInput>,
    /// The name and description of the project the input holds, as last
    /// read from (or written to) the database.
    saved: Option<(String, String)>,
    autosave: Option<Task<()>>,
    /// It now holds another project's description: if it has focus, the
    /// next render gives the focus back to the list, so that typing doesn't
    /// edit the new project unawares.
    switched: bool,
}

impl ProjectDescription {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.weak_entity();
        let input = cx.new(|cx| {
            TextInput::new("Add a description…", cx)
                .multiline()
                .links(move |text, cx| {
                    workspace
                        .upgrade()
                        .map(|w| w.read(cx).description_links(text))
                        .unwrap_or_default()
                })
        });
        cx.subscribe_in(&input, window, |this, _, _: &Changed, window, cx| {
            this.schedule_project_autosave(window, cx)
        })
        .detach();
        // Blur listeners run while the window draws: save right after.
        cx.on_blur(&input.focus_handle(cx), window, |_, window, cx| {
            cx.defer_in(window, |this, _, cx| this.save_project_description(cx))
        })
        .detach();
        Self {
            input,
            saved: None,
            autosave: None,
            switched: false,
        }
    }

    /// Follows a rename of the project it holds, keeping unsaved edits.
    pub(super) fn renamed(&mut self, old: &str, new: &str) {
        if let Some((name, _)) = &mut self.saved
            && name == old
        {
            *name = new.to_owned();
        }
    }
}

impl Workspace {
    fn schedule_project_autosave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_description.autosave = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            this.update(cx, |this, cx| this.save_project_description(cx))
                .ok();
        }));
    }

    /// Writes the edited description, if changed.
    pub(super) fn save_project_description(&mut self, cx: &mut Context<Self>) {
        let field = &self.project_description;
        let Some((name, saved)) = &field.saved else {
            return;
        };
        let text = field.input.read(cx).text();
        if !description_changed(text, saved) {
            return;
        }
        let update = ProjectUpdate {
            description: Some(text.to_owned()),
            ..Default::default()
        };
        if let Some(project) = self.report(self.db.update_project(name, &update), cx) {
            self.project_description.saved = Some((project.name, project.description));
            self.reload(cx);
        }
    }

    /// After a reload: shows the description of the project now shown.
    /// While it is the same project, unsaved edits are kept (as in the
    /// issue detail); another project replaces the text.
    pub(super) fn refresh_project_description(&mut self, cx: &mut Context<Self>) {
        let shown = self
            .shown_project()
            .map(|p| (p.name.clone(), p.description.clone()));
        let field = &mut self.project_description;
        let same = matches!((&field.saved, &shown), (Some((a, _)), Some((b, _))) if a == b);
        let text = field.input.read(cx).text();
        let reset = match (&field.saved, &shown) {
            (Some((_, saved)), Some((_, new))) if same => (!description_changed(text, saved)
                && description_changed(text, new))
            .then(|| new.clone()),
            _ => Some(shown.as_ref().map(|(_, d)| d.clone()).unwrap_or_default()),
        };
        if let Some(text) = reset {
            if !same {
                field.autosave = None;
                field.switched = true;
            }
            field
                .input
                .update(cx, |input, cx| input.reset_text(text, cx));
        }
        field.saved = shown;
    }

    /// Before rendering: see [`ProjectDescription::switched`].
    pub(super) fn sync_project_description(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = &mut self.project_description;
        if std::mem::take(&mut field.switched) && field.input.focus_handle(cx).is_focused(window) {
            window.focus(&self.list_focus);
        }
    }

    pub(super) fn on_edit_project_description(
        &mut self,
        _: &project::EditDescription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shown_project().is_none() {
            self.report::<()>(Err(anyhow::anyhow!("Not showing a project")), cx);
            return;
        }
        // The header is above the list, which these replace.
        self.close_detail(cx);
        if self.showing_archive {
            self.close_archive(window, cx);
        }
        let input = self.project_description.input.clone();
        window.focus(&input.focus_handle(cx));
        input.update(cx, |input, cx| input.move_to_end(cx));
    }

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
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let status = project.status;
        let status_chip = div()
            .id("project-status")
            .ml_3()
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
                // Margins, not a gap: the row's gap doesn't show next to the
                // name.
                div()
                    .flex()
                    .items_center()
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
            .child(self.render_project_description(window, cx))
    }

    /// The description field, cut to a few lines while not edited if long.
    fn render_project_description(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let input = &self.project_description.input;
        let editing = input.focus_handle(cx).is_focused(window);
        let text = input.read(cx).text();
        // Wrapped lines aren't known here: count a line per 100 characters.
        let lines: usize = text.lines().map(|l| 1 + l.len() / 100).sum();
        let collapsed = !editing && lines > COLLAPSED_LINES;
        div()
            .flex()
            .flex_col()
            .text_color(rgb(theme::TEXT_MUTED))
            .line_height(px(LINE_HEIGHT))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(editing, |s| s.text_color(rgb(theme::TEXT)))
                    .when(collapsed, |s| {
                        s.max_h(px(LINE_HEIGHT * COLLAPSED_LINES as f32))
                            .overflow_hidden()
                    })
                    .child(input.clone()),
            )
            .when(collapsed, |s| {
                s.child(
                    div()
                        .id("project-description-more")
                        .flex_none()
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(theme::TEXT)))
                        .child("Show more")
                        .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                            window.dispatch_action(project::EditDescription.boxed_clone(), cx);
                        })),
                )
            })
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
        .pl_3()
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
