//! The main window: sidebar, new-issue input, issue list, modals (command
//! palette, pickers) and an error bar.
//!
//! Every user-facing behavior here is an action handler (see `actions.rs`);
//! mouse handlers only select things and dispatch the same actions.

mod filter;
mod scope;
mod sidebar;

use std::time::Duration;

use gpui::{
    Action, App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    ScrollHandle, Subscription, Task, Window, deferred, div, prelude::*, px, rgb,
};
use solilog_core::{Db, Issue, IssueFilter, IssueId, Label, NewProject, Project, Status, View};

use super::actions::{
    self, command_palette, humanize_action_name,
    issue::{self, AddLabel, AddToProject, RemoveFromProject, RemoveLabel, SetStatus},
    issue_list, view,
    workspace::{OpenKeymap, ReloadKeymap},
};
use super::keymap;
use super::picker::{Picker, PickerEvent, PickerItem};
use super::status_icon::status_icon;
use super::text_input::{Submit, TextInput};
use super::theme;
use scope::Scope;
use sidebar::SidebarItem;

/// How often to check the database (and keymap file) for outside changes.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

enum Modal {
    CommandPalette {
        picker: Entity<Picker>,
        actions: Vec<Box<dyn Action>>,
    },
    StatusPicker {
        picker: Entity<Picker>,
        issue_id: IssueId,
    },
    Relation {
        picker: Entity<Picker>,
        issue_id: IssueId,
        relation: Relation,
        /// Label or project names, by picker index.
        names: Vec<String>,
    },
    /// A picker whose items each dispatch an action (open view, project…).
    ActionPicker {
        picker: Entity<Picker>,
        actions: Vec<Box<dyn Action>>,
        /// Builds the action for the "Create …" row from the typed text.
        create: Option<CreateAction>,
    },
}

type CreateAction = Box<dyn Fn(String) -> Box<dyn Action>>;

/// A change to an issue's labels or projects, each an action that picks the
/// label or project interactively when not given one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    AddLabel,
    RemoveLabel,
    AddToProject,
    RemoveFromProject,
}

impl Relation {
    fn placeholder(self) -> &'static str {
        match self {
            Relation::AddLabel => "Add label…",
            Relation::RemoveLabel => "Remove label…",
            Relation::AddToProject => "Add to project…",
            Relation::RemoveFromProject => "Remove from project…",
        }
    }

    /// Prefix of the picker's "Create …" row, for relations that can create.
    fn create_label(self) -> Option<&'static str> {
        match self {
            Relation::AddLabel => Some("Create label"),
            Relation::AddToProject => Some("Create project"),
            Relation::RemoveLabel | Relation::RemoveFromProject => None,
        }
    }
}

impl Modal {
    fn picker(&self) -> &Entity<Picker> {
        match self {
            Modal::CommandPalette { picker, .. }
            | Modal::StatusPicker { picker, .. }
            | Modal::Relation { picker, .. }
            | Modal::ActionPicker { picker, .. } => picker,
        }
    }
}

struct OpenModal {
    modal: Modal,
    /// Focus to restore when the modal closes.
    restore_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

pub struct Workspace {
    db: Db,
    /// Cache of the database; reloaded whenever it changes. Newest first.
    issues: Vec<Issue>,
    /// Every issue, for the sidebar's counts.
    all_issues: Vec<Issue>,
    views: Vec<View>,
    /// Including done and archived ones.
    projects: Vec<Project>,
    labels: Vec<Label>,
    data_version: i64,
    /// What the issue list shows. Re-read on every reload, so outside
    /// edits (e.g. from the CLI) apply.
    scope: Scope,
    selected: Option<IssueId>,
    input: Entity<TextInput>,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    sidebar_visible: bool,
    sidebar_focus: FocusHandle,
    /// Keyboard cursor in the sidebar.
    sidebar_cursor: SidebarItem,
    show_closed_projects: bool,
    modal: Option<OpenModal>,
    error: Option<String>,
    _poll: Task<()>,
}

impl Workspace {
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("New issue…", cx));
        cx.subscribe_in(&input, window, |this, input, _: &Submit, _, cx| {
            let title = input.read(cx).text().to_owned();
            if title.trim().is_empty() {
                return;
            }
            // On failure the text stays in the input so nothing is lost.
            if let Some(issue) = this.report(this.db.create_issue(&this.new_issue(title)), cx) {
                input.update(cx, |input, cx| input.clear(cx));
                this.selected = Some(issue.id);
                this.reload(cx);
            }
        })
        .detach();

        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    keymap::reload_if_changed(cx);
                    if this.db.data_version().ok() != Some(this.data_version) {
                        this.reload(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            db,
            issues: Vec::new(),
            all_issues: Vec::new(),
            views: Vec::new(),
            projects: Vec::new(),
            labels: Vec::new(),
            data_version: 0,
            scope: Scope::default(),
            selected: None,
            input,
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            sidebar_visible: true,
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: SidebarItem::AllIssues,
            show_closed_projects: false,
            modal: None,
            error: None,
            _poll: poll,
        };
        this.reload(cx);
        if this.issues.is_empty() {
            window.focus(&this.input.focus_handle(cx));
        } else {
            window.focus(&this.list_focus);
        }
        this
    }

    /// Records an error for display; returns the success value otherwise.
    fn report<T>(&mut self, result: anyhow::Result<T>, cx: &mut Context<Self>) -> Option<T> {
        match result {
            Ok(value) => {
                self.error = None;
                Some(value)
            }
            Err(err) => {
                self.error = Some(format!("{err:#}"));
                cx.notify();
                None
            }
        }
    }

    /// Re-reads everything from the database (the source of truth).
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.data_version = self.db.data_version().unwrap_or_default();
        let prev_ix = self.selected_index();
        let scope_error = self.refresh_scope();
        let filter = self.scope.filter.clone();
        let sidebar_data = self
            .db
            .list_issues(&IssueFilter::default())
            .and_then(|issues| {
                Ok((
                    issues,
                    self.db.list_views()?,
                    self.db.list_projects(true)?,
                    self.db.list_labels()?,
                ))
            });
        if let Some((all_issues, views, projects, labels)) = self.report(sidebar_data, cx) {
            self.all_issues = all_issues;
            self.views = views;
            self.projects = projects;
            self.labels = labels;
        }
        let Some(issues) = self.report(self.db.list_issues(&filter), cx) else {
            self.issues.clear();
            return;
        };
        self.issues = issues;
        if scope_error.is_some() {
            self.error = scope_error;
        }
        // Keep the selection on the same issue, or on the same row if it vanished.
        if self.selected_index().is_none() {
            let ix = prev_ix
                .unwrap_or(0)
                .min(self.issues.len().saturating_sub(1));
            self.selected = self.issues.get(ix).map(|i| i.id);
        }
        cx.notify();
    }

    fn selected_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.issues.iter().position(|i| i.id == id)
    }

    fn select_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(issue) = self.issues.get(ix) {
            self.selected = Some(issue.id);
            self.list_scroll.scroll_to_item(ix);
            cx.notify();
        }
    }

    // ---- modals ---------------------------------------------------------

    fn open_modal(
        &mut self,
        picker: Entity<Picker>,
        make: impl FnOnce(Entity<Picker>) -> Modal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let restore_focus = window.focused(cx);
        let subscription = cx.subscribe_in(&picker, window, Self::on_picker_event);
        window.focus(&picker.focus_handle(cx));
        self.modal = Some(OpenModal {
            modal: make(picker),
            restore_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Modal> {
        let open = self.modal.take()?;
        let focus = open
            .restore_focus
            .filter(|f| *f != open.modal.picker().focus_handle(cx))
            .unwrap_or_else(|| self.list_focus.clone());
        window.focus(&focus);
        cx.notify();
        Some(open.modal)
    }

    fn on_picker_event(
        &mut self,
        _: &Entity<Picker>,
        event: &PickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.close_modal(window, cx) else {
            return;
        };
        match (modal, event) {
            (_, PickerEvent::Dismissed) => {}
            (Modal::CommandPalette { actions, .. }, PickerEvent::Confirmed(ix)) => {
                // Focus has been restored, so the action reaches the same
                // handlers it would have reached from a key binding.
                window.dispatch_action(actions[*ix].boxed_clone(), cx);
            }
            (Modal::StatusPicker { issue_id, .. }, PickerEvent::Confirmed(ix)) => {
                self.set_status(issue_id, Status::ALL[*ix], cx);
            }
            (
                Modal::Relation {
                    issue_id,
                    relation,
                    names,
                    ..
                },
                PickerEvent::Confirmed(ix),
            ) => {
                self.apply_relation(relation, issue_id, names[*ix].clone(), cx);
            }
            (
                Modal::Relation {
                    issue_id, relation, ..
                },
                PickerEvent::Create(name),
            ) => {
                if relation == Relation::AddToProject
                    && self
                        .report(self.db.create_project(&NewProject::new(name.clone())), cx)
                        .is_none()
                {
                    return;
                }
                self.apply_relation(relation, issue_id, name.clone(), cx);
            }
            (Modal::ActionPicker { actions, .. }, PickerEvent::Confirmed(ix)) => {
                window.dispatch_action(actions[*ix].boxed_clone(), cx);
            }
            (
                Modal::ActionPicker {
                    create: Some(create),
                    ..
                },
                PickerEvent::Create(text),
            ) => {
                window.dispatch_action(create(text.clone()), cx);
            }
            (_, PickerEvent::Create(_)) => {}
        }
    }

    fn toggle_command_palette(
        &mut self,
        _: &command_palette::Toggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.modal,
            Some(OpenModal {
                modal: Modal::CommandPalette { .. },
                ..
            })
        ) {
            self.close_modal(window, cx);
            return;
        }
        self.close_modal(window, cx);

        let mut entries: Vec<(String, Box<dyn Action>)> = window
            .available_actions(cx)
            .into_iter()
            .filter(|a| actions::is_palette_visible(a.name()))
            .map(|a| (humanize_action_name(a.name()), a))
            .collect();
        // Every saved view is a command of its own.
        if entries.iter().any(|(_, a)| a.name() == "view::Open")
            && let Ok(views) = self.db.list_views()
        {
            entries.extend(views.into_iter().map(|v| {
                let label = format!("View: Open › {}", v.name);
                let action = view::Open { name: Some(v.name) };
                (label, Box::new(action) as Box<dyn Action>)
            }));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let items = entries
            .iter()
            .map(|(label, action)| {
                let hint = window
                    .highest_precedence_binding_for_action(action.as_ref())
                    .map(|b| keystrokes_text(&b));
                PickerItem::new(label.clone()).hint(hint)
            })
            .collect();
        let actions = entries.into_iter().map(|(_, a)| a).collect();
        let picker = cx.new(|cx| Picker::new("Run a command…", items, 0, cx));
        self.open_modal(
            picker,
            |picker| Modal::CommandPalette { picker, actions },
            window,
            cx,
        );
    }

    fn open_status_picker(&mut self, issue: &Issue, window: &mut Window, cx: &mut Context<Self>) {
        let items = Status::ALL
            .into_iter()
            .map(|s| PickerItem::new(s.label()).icon(move || status_icon(s)))
            .collect();
        let current = Status::ALL.iter().position(|s| *s == issue.status);
        let picker = cx.new(|cx| Picker::new("Change status…", items, current.unwrap_or(0), cx));
        let issue_id = issue.id;
        self.open_modal(
            picker,
            |picker| Modal::StatusPicker { picker, issue_id },
            window,
            cx,
        );
    }

    fn open_relation_picker(
        &mut self,
        relation: Relation,
        issue: &Issue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let not_on_issue = |current: &[String], all: Vec<String>| -> Vec<String> {
            all.into_iter()
                .filter(|name| !current.iter().any(|c| c.eq_ignore_ascii_case(name)))
                .collect()
        };
        let names = match relation {
            Relation::AddLabel => {
                let Some(all) = self.report(self.db.list_labels(), cx) else {
                    return;
                };
                not_on_issue(&issue.labels, all.into_iter().map(|l| l.name).collect())
            }
            Relation::AddToProject => {
                let Some(all) = self.report(self.db.list_projects(false), cx) else {
                    return;
                };
                not_on_issue(&issue.projects, all.into_iter().map(|p| p.name).collect())
            }
            Relation::RemoveLabel => issue.labels.clone(),
            Relation::RemoveFromProject => issue.projects.clone(),
        };
        if names.is_empty() && relation.create_label().is_none() {
            let what = match relation {
                Relation::RemoveLabel => "has no labels",
                _ => "is in no project",
            };
            self.error = Some(format!("{} {what}", issue.id));
            cx.notify();
            return;
        }
        let items = names.iter().map(|n| PickerItem::new(n.clone())).collect();
        let picker = cx.new(|cx| {
            let picker = Picker::new(relation.placeholder(), items, 0, cx);
            match relation.create_label() {
                Some(label) => picker.creatable(label),
                None => picker,
            }
        });
        let issue_id = issue.id;
        self.open_modal(
            picker,
            |picker| Modal::Relation {
                picker,
                issue_id,
                relation,
                names,
            },
            window,
            cx,
        );
    }

    // ---- domain actions ---------------------------------------------------

    fn selected_issue(&mut self, cx: &mut Context<Self>) -> Option<Issue> {
        let issue = self.selected_index().map(|ix| self.issues[ix].clone());
        if issue.is_none() {
            self.error = Some("No issue selected".into());
            cx.notify();
        }
        issue
    }

    fn apply_relation(
        &mut self,
        relation: Relation,
        id: IssueId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let names = [name];
        let result = match relation {
            Relation::AddLabel => self.db.add_labels(id, &names),
            Relation::RemoveLabel => self.db.remove_labels(id, &names),
            Relation::AddToProject => self.db.add_to_projects(id, &names),
            Relation::RemoveFromProject => self.db.remove_from_projects(id, &names),
        };
        if self.report(result, cx).is_some() {
            self.reload(cx);
        }
    }

    /// Applies `relation` to the selected issue, or opens a picker when no
    /// name is given.
    fn on_relation(
        &mut self,
        relation: Relation,
        name: Option<&String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(issue) = self.selected_issue(cx) else {
            return;
        };
        match name {
            Some(name) => self.apply_relation(relation, issue.id, name.clone(), cx),
            None => self.open_relation_picker(relation, &issue, window, cx),
        }
    }

    fn on_add_label(&mut self, action: &AddLabel, window: &mut Window, cx: &mut Context<Self>) {
        self.on_relation(Relation::AddLabel, action.label.as_ref(), window, cx);
    }

    fn on_remove_label(
        &mut self,
        action: &RemoveLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_relation(Relation::RemoveLabel, action.label.as_ref(), window, cx);
    }

    fn on_add_to_project(
        &mut self,
        action: &AddToProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_relation(Relation::AddToProject, action.project.as_ref(), window, cx);
    }

    fn on_remove_from_project(
        &mut self,
        action: &RemoveFromProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_relation(
            Relation::RemoveFromProject,
            action.project.as_ref(),
            window,
            cx,
        );
    }

    fn set_status(&mut self, id: IssueId, status: Status, cx: &mut Context<Self>) {
        if self.report(self.db.set_status(id, status), cx).is_some() {
            self.reload(cx);
        }
    }

    fn on_set_status(&mut self, action: &SetStatus, window: &mut Window, cx: &mut Context<Self>) {
        let Some(issue) = self.selected_issue(cx) else {
            return;
        };
        match action.status {
            Some(status) => self.set_status(issue.id, status, cx),
            None => self.open_status_picker(&issue, window, cx),
        }
    }

    fn on_new_issue(&mut self, _: &issue::New, window: &mut Window, cx: &mut Context<Self>) {
        self.close_modal(window, cx);
        window.focus(&self.input.focus_handle(cx));
    }

    fn on_open_keymap(&mut self, _: &OpenKeymap, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.report(keymap::ensure_user_keymap(), cx) {
            cx.open_with_system(&path);
        }
    }

    fn on_reload_keymap(&mut self, _: &ReloadKeymap, _: &mut Window, cx: &mut Context<Self>) {
        keymap::load(cx);
    }

    // ---- list navigation --------------------------------------------------

    fn on_focus_list(
        &mut self,
        _: &issue_list::Focus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.list_focus);
        cx.notify();
    }

    fn select_next(&mut self, _: &issue_list::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected_index().map_or(0, |ix| ix + 1);
        self.select_index(ix.min(self.issues.len().saturating_sub(1)), cx);
    }

    fn select_previous(
        &mut self,
        _: &issue_list::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.selected_index().map_or(0, |ix| ix.saturating_sub(1));
        self.select_index(ix, cx);
    }

    fn select_first(
        &mut self,
        _: &issue_list::SelectFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_index(0, cx);
    }

    fn select_last(&mut self, _: &issue_list::SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.select_index(self.issues.len().saturating_sub(1), cx);
    }

    // ---- rendering --------------------------------------------------------

    fn render_issue(
        &self,
        ix: usize,
        issue: &Issue,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let issue_id = issue.id;
        let selected = self.selected == Some(issue_id);

        let status = div()
            .id(("status", issue_id.0 as u64))
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::HIGHLIGHT)))
            .child(status_icon(issue.status))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_index(ix, cx);
                window.focus(&this.list_focus);
                window.dispatch_action(SetStatus::default().boxed_clone(), cx);
            }));

        div()
            .id(("issue", issue_id.0 as u64))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_1()
            .when(selected, |s| {
                s.bg(rgb(if list_focused {
                    theme::HIGHLIGHT
                } else {
                    theme::HOVER
                }))
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_index(ix, cx);
                window.focus(&this.list_focus);
            }))
            .child(status)
            .child(
                div()
                    .w(px(52.))
                    .flex_none()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(issue_id.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(issue.status.is_closed(), |s| {
                        s.text_color(rgb(theme::TEXT_MUTED))
                    })
                    .child(issue.title.clone()),
            )
            .children(
                issue
                    .projects
                    .iter()
                    .map(|project| chip(project.clone(), theme::ACCENT)),
            )
            .children(
                issue
                    .labels
                    .iter()
                    .map(|label| chip(label.clone(), theme::TEXT_MUTED)),
            )
    }

    fn render_modal(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let picker = self.modal.as_ref()?.modal.picker().clone();
        let dismiss = cx.listener(|this, _, window, cx| {
            this.close_modal(window, cx);
        });
        Some(
            deferred(
                div()
                    .id("modal-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .justify_center()
                    .pt(px(80.))
                    .on_mouse_down(MouseButton::Left, dismiss)
                    // Clicks inside the picker must not reach the backdrop.
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(picker),
                    ),
            )
            .with_priority(1),
        )
    }
}

/// A small rounded tag, used for projects and labels.
fn chip(text: String, color: u32) -> impl IntoElement {
    div()
        .flex_none()
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(rgb(color))
        .child(text)
}

/// "g g" -> "G G", "cmd-k" -> "⌘K" (platform display form).
fn keystrokes_text(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list_focused = self.list_focus.is_focused(window);
        div()
            .key_context("Workspace")
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::on_new_issue))
            .on_action(cx.listener(Self::on_set_status))
            .on_action(cx.listener(Self::on_add_label))
            .on_action(cx.listener(Self::on_remove_label))
            .on_action(cx.listener(Self::on_add_to_project))
            .on_action(cx.listener(Self::on_remove_from_project))
            .on_action(cx.listener(Self::on_open_view))
            .on_action(cx.listener(Self::on_show_all))
            .on_action(cx.listener(Self::on_open_project))
            .on_action(cx.listener(Self::on_open_label))
            .on_action(cx.listener(Self::on_toggle_closed_projects))
            .on_action(cx.listener(Self::on_move_project))
            .on_action(cx.listener(Self::on_toggle_label_filter))
            .on_action(cx.listener(Self::on_exclude_label_filter))
            .on_action(cx.listener(Self::on_toggle_project_filter))
            .on_action(cx.listener(Self::on_toggle_status_filter))
            .on_action(cx.listener(Self::on_toggle_open_only))
            .on_action(cx.listener(Self::on_toggle_no_project))
            .on_action(cx.listener(Self::on_clear_filter))
            .on_action(cx.listener(Self::on_save_view))
            .on_action(cx.listener(Self::on_save_view_as))
            .on_action(cx.listener(Self::on_revert_view))
            .on_action(cx.listener(Self::on_rename_view))
            .on_action(cx.listener(Self::on_delete_view))
            .on_action(cx.listener(Self::on_focus_list))
            .on_action(cx.listener(Self::on_open_keymap))
            .on_action(cx.listener(Self::on_reload_keymap))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_focus_sidebar))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .text_size(px(14.))
            .line_height(px(20.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(self.render_sidebar(window, cx))
                    .child(self.render_main(list_focused, cx)),
            )
            .children(self.error.clone().map(|err| {
                div()
                    .px_4()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(theme::DIVIDER))
                    .text_color(rgb(theme::ERROR))
                    .child(err)
            }))
            .children(self.render_modal(cx))
    }
}

impl Workspace {
    /// New-issue input, view header and issue list.
    fn render_main(&self, list_focused: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows: Vec<_> = self
            .issues
            .iter()
            .enumerate()
            .map(|(ix, issue)| {
                self.render_issue(ix, issue, list_focused, cx)
                    .into_any_element()
            })
            .collect();
        let empty = self.issues.is_empty();
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .key_context("NewIssue")
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(theme::DIVIDER))
                    .child(self.input.clone()),
            )
            .child(self.render_filter_header(cx))
            .child(
                div()
                    .id("issues")
                    .key_context("IssueList")
                    .track_focus(&self.list_focus)
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::select_previous))
                    .on_action(cx.listener(Self::select_first))
                    .on_action(cx.listener(Self::select_last))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .py_1()
                    .children(rows)
                    .when(empty, |s| {
                        s.child(
                            div()
                                .px_4()
                                .py_2()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(if self.scope.source != scope::Source::All {
                                    "No issues here."
                                } else {
                                    "No issues yet. Type a title above and press Enter."
                                }),
                        )
                    }),
            )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}
