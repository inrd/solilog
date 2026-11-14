//! The main window: sidebar, new-issue input, issue list (or the archive, or
//! an open issue), modals (command palette, pickers) and an error bar.
//!
//! Every user-facing behavior here is an action handler (see `actions.rs`);
//! mouse handlers only select things and dispatch the same actions.

mod archive;
mod detail;
mod filter;
mod manage;
mod scope;
mod sidebar;

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use gpui::{
    Action, App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, Pixels, Point, ScrollHandle, Subscription, Task, Window, anchored, deferred,
    div, prelude::*, px, rgb,
};
use solilog_core::{
    Db, Issue, IssueChange, IssueEdit, IssueFilter, IssueId, Label, NewProject, Project, Status,
    View,
};

use super::actions::{
    self, command_palette, humanize_action_name,
    issue::{self, AddLabel, AddToProject, Edit, RemoveFromProject, RemoveLabel, SetStatus},
    issue_list, view,
    workspace::{OpenKeymap, ReloadKeymap},
};
use super::keymap;
use super::picker::{Picker, PickerEvent, PickerItem};
use super::status_icon::status_icon;
use super::text_input::{Submit, TextInput};
use super::theme;
use archive::ArchiveItem;
use detail::IssueDetail;
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
        issue_ids: Vec<IssueId>,
    },
    Relation {
        picker: Entity<Picker>,
        issue_ids: Vec<IssueId>,
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
    /// The actions of a sidebar row or archived item.
    ContextMenu {
        picker: Entity<Picker>,
        actions: Vec<Box<dyn Action>>,
        target: MenuTarget,
        /// Where it was opened with the mouse (window coordinates); centered
        /// like other pickers otherwise.
        position: Option<Point<Pixels>>,
    },
}

type CreateAction = Box<dyn Fn(String) -> Box<dyn Action>>;

/// A context menu entry: its label and the action it dispatches.
type MenuEntry = (&'static str, Box<dyn Action>);

/// What a context menu is about (highlighted while the menu is open).
#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuTarget {
    Sidebar(SidebarItem),
    Archive(ArchiveItem),
    Issue(IssueId),
}

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

    fn change(self, name: String) -> IssueChange {
        let names = vec![name];
        match self {
            Relation::AddLabel => IssueChange::AddLabels(names),
            Relation::RemoveLabel => IssueChange::RemoveLabels(names),
            Relation::AddToProject => IssueChange::AddToProjects(names),
            Relation::RemoveFromProject => IssueChange::RemoveFromProjects(names),
        }
    }
}

impl Modal {
    fn picker(&self) -> &Entity<Picker> {
        match self {
            Modal::CommandPalette { picker, .. }
            | Modal::StatusPicker { picker, .. }
            | Modal::Relation { picker, .. }
            | Modal::ActionPicker { picker, .. }
            | Modal::ContextMenu { picker, .. } => picker,
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
    /// Every issue but archived ones, for the sidebar's counts.
    all_issues: Vec<Issue>,
    /// Newest first.
    archived_issues: Vec<Issue>,
    views: Vec<View>,
    /// Including done and archived ones.
    projects: Vec<Project>,
    labels: Vec<Label>,
    data_version: i64,
    /// What the issue list shows. Re-read on every reload, so outside
    /// edits (e.g. from the CLI) apply.
    scope: Scope,
    /// The highlighted issue (the keyboard cursor).
    selected: Option<IssueId>,
    /// Issues selected for commands, which act on these when there are any
    /// and on the highlighted issue otherwise. Only ever listed issues:
    /// reloads drop the others.
    selection: HashSet<IssueId>,
    /// Where the range being selected (shift + move or click) started.
    selection_anchor: Option<IssueId>,
    input: Entity<TextInput>,
    list_focus: FocusHandle,
    list_scroll: ScrollHandle,
    sidebar_visible: bool,
    sidebar_focus: FocusHandle,
    /// Keyboard cursor in the sidebar.
    sidebar_cursor: SidebarItem,
    show_closed_projects: bool,
    /// The archive is shown in place of the issue list; `scope` is kept for
    /// when it closes.
    showing_archive: bool,
    archive_cursor: Option<ArchiveItem>,
    archive_scroll: ScrollHandle,
    /// The open issue, shown in place of the list (or the archive).
    detail: Option<IssueDetail>,
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

        // When the focused element goes away (e.g. the open issue was
        // deleted), focus the list.
        cx.on_focus_lost(window, |this, window, _| window.focus(&this.list_focus))
            .detach();
        cx.on_app_quit(|this, cx| {
            this.save_detail(false, cx);
            async {}
        })
        .detach();

        let mut this = Self {
            db,
            issues: Vec::new(),
            all_issues: Vec::new(),
            archived_issues: Vec::new(),
            views: Vec::new(),
            projects: Vec::new(),
            labels: Vec::new(),
            data_version: 0,
            scope: Scope::default(),
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            input,
            list_focus: cx.focus_handle(),
            list_scroll: ScrollHandle::new(),
            sidebar_visible: true,
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: SidebarItem::AllIssues,
            show_closed_projects: false,
            showing_archive: false,
            archive_cursor: None,
            archive_scroll: ScrollHandle::new(),
            detail: None,
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
        let prev_archive_ix = self.archive_index();
        let scope_error = self.refresh_scope();
        let filter = self.scope.filter.clone();
        let sidebar_data = self
            .db
            .list_issues(&IssueFilter::default())
            .and_then(|issues| {
                Ok((
                    issues,
                    self.db.list_archived_issues(&IssueFilter::default())?,
                    self.db.list_views()?,
                    self.db.list_projects(true)?,
                    self.db.list_labels()?,
                ))
            });
        if let Some((all_issues, archived_issues, views, projects, labels)) =
            self.report(sidebar_data, cx)
        {
            self.all_issues = all_issues;
            self.archived_issues = archived_issues;
            self.views = views;
            self.projects = projects;
            self.labels = labels;
        }
        self.fix_archive_cursor(prev_archive_ix);
        self.refresh_detail(cx);
        let Some(issues) = self.report(self.db.list_issues(&filter), cx) else {
            self.issues.clear();
            return;
        };
        self.issues = issues;
        let listed: HashSet<IssueId> = self.issues.iter().map(|i| i.id).collect();
        self.selection.retain(|id| listed.contains(id));
        self.selection_anchor = self.selection_anchor.filter(|id| listed.contains(id));
        if scope_error.is_some() {
            self.error = scope_error;
        }
        if !self.sidebar_items().contains(&self.sidebar_cursor) {
            self.sidebar_cursor = self.active_sidebar_item();
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

    /// Highlights the issue at `ix`; a later range selection starts there.
    fn select_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selection_anchor = None;
        self.highlight_index(ix, cx);
    }

    fn highlight_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(issue) = self.issues.get(ix) {
            self.selected = Some(issue.id);
            self.list_scroll.scroll_to_item(ix);
            cx.notify();
        }
    }

    /// Moves the highlight to `ix`, selecting the issues from the anchor
    /// (where the range started) to it, instead of the previous range.
    fn extend_selection_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(cursor) = self.selected_index() else {
            return self.select_index(ix, cx);
        };
        let anchor = self
            .selection_anchor
            .and_then(|id| self.issues.iter().position(|i| i.id == id))
            .unwrap_or(cursor);
        let ix = ix.min(self.issues.len().saturating_sub(1));
        let range = |end: usize| anchor.min(end)..=anchor.max(end);
        for issue in &self.issues[range(cursor)] {
            self.selection.remove(&issue.id);
        }
        for issue in &self.issues[range(ix)] {
            self.selection.insert(issue.id);
        }
        self.selection_anchor = Some(self.issues[anchor].id);
        self.highlight_index(ix, cx);
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
            (Modal::StatusPicker { issue_ids, .. }, PickerEvent::Confirmed(ix)) => {
                self.set_status(&issue_ids, Status::ALL[*ix], cx);
            }
            (
                Modal::Relation {
                    issue_ids,
                    relation,
                    names,
                    ..
                },
                PickerEvent::Confirmed(ix),
            ) => {
                self.apply_relation(relation, &issue_ids, names[*ix].clone(), cx);
            }
            (
                Modal::Relation {
                    issue_ids,
                    relation,
                    ..
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
                self.apply_relation(relation, &issue_ids, name.clone(), cx);
            }
            (
                Modal::ActionPicker { actions, .. } | Modal::ContextMenu { actions, .. },
                PickerEvent::Confirmed(ix),
            ) => {
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
        // Hints are the bindings that would run the action from where the
        // palette was opened (bindings can be specific to nested contexts).
        let focused = window.focused(cx);
        let items = entries
            .iter()
            .map(|(label, action)| {
                let binding = match &focused {
                    Some(focus) => {
                        window.highest_precedence_binding_for_action_in(action.as_ref(), focus)
                    }
                    None => window.highest_precedence_binding_for_action(action.as_ref()),
                };
                let hint = binding.map(|b| keystrokes_text(&b));
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

    fn open_status_picker(
        &mut self,
        issues: &[Issue],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = Status::ALL
            .into_iter()
            .map(|s| PickerItem::new(s.label()).icon(move || status_icon(s)))
            .collect();
        // The issues' status, if they share one.
        let current = Status::ALL
            .iter()
            .position(|s| issues.iter().all(|i| i.status == *s));
        let picker = cx.new(|cx| {
            Picker::new(
                &format!("Change status{}…", of_issues(issues)),
                items,
                current.unwrap_or(0),
                cx,
            )
        });
        let issue_ids = issues.iter().map(|i| i.id).collect();
        self.open_modal(
            picker,
            |picker| Modal::StatusPicker { picker, issue_ids },
            window,
            cx,
        );
    }

    fn open_relation_picker(
        &mut self,
        relation: Relation,
        issues: &[Issue],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        fn labels(issue: &Issue) -> &[String] {
            &issue.labels
        }
        fn projects(issue: &Issue) -> &[String] {
            &issue.projects
        }
        type Names = fn(&Issue) -> &[String];
        // Names to add: those not every issue has already.
        let not_on_all = |all: Vec<String>, current: Names| {
            all.into_iter()
                .filter(|name| {
                    !issues
                        .iter()
                        .all(|i| current(i).iter().any(|c| c.eq_ignore_ascii_case(name)))
                })
                .collect::<Vec<_>>()
        };
        // Names to remove: those any issue has.
        let on_any = |current: Names| {
            let names: BTreeSet<_> = issues.iter().flat_map(|i| current(i).to_vec()).collect();
            names.into_iter().collect::<Vec<_>>()
        };
        let names = match relation {
            Relation::AddLabel => {
                let Some(all) = self.report(self.db.list_labels(), cx) else {
                    return;
                };
                not_on_all(all.into_iter().map(|l| l.name).collect(), labels)
            }
            Relation::AddToProject => {
                let Some(all) = self.report(self.db.list_projects(false), cx) else {
                    return;
                };
                not_on_all(all.into_iter().map(|p| p.name).collect(), projects)
            }
            Relation::RemoveLabel => on_any(labels),
            Relation::RemoveFromProject => on_any(projects),
        };
        if names.is_empty() && relation.create_label().is_none() {
            let what = match relation {
                Relation::RemoveLabel => "no labels",
                _ => "no projects",
            };
            self.error = Some(match issues {
                [issue] => format!("{} has {what}", issue.id),
                _ => format!("The selected issues have {what}"),
            });
            cx.notify();
            return;
        }
        let items = names.iter().map(|n| PickerItem::new(n.clone())).collect();
        let placeholder = relation
            .placeholder()
            .replace('…', &format!("{}…", of_issues(issues)));
        let picker = cx.new(|cx| {
            let picker = Picker::new(&placeholder, items, 0, cx);
            match relation.create_label() {
                Some(label) => picker.creatable(label),
                None => picker,
            }
        });
        let issue_ids = issues.iter().map(|i| i.id).collect();
        self.open_modal(
            picker,
            |picker| Modal::Relation {
                picker,
                issue_ids,
                relation,
                names,
            },
            window,
            cx,
        );
    }

    // ---- domain actions ---------------------------------------------------

    /// The selected issue: the open one, else the highlighted one (in the
    /// archive, the highlighted archived issue).
    fn selected_issue(&mut self, cx: &mut Context<Self>) -> Option<Issue> {
        let issue = if let Some(detail) = &self.detail {
            Some(detail.issue().clone())
        } else if self.showing_archive {
            self.archive_selected_issue()
        } else {
            self.selected_index().map(|ix| self.issues[ix].clone())
        };
        if issue.is_none() {
            self.error = Some("No issue selected".into());
            cx.notify();
        }
        issue
    }

    /// What issue commands act on: the open issue, else the selected
    /// issues, in list order, or else the highlighted one (in the archive,
    /// the highlighted archived issue). Empty, with an error shown, when
    /// there is none.
    fn target_issues(&mut self, cx: &mut Context<Self>) -> Vec<Issue> {
        if self.detail.is_none() && !self.showing_archive && !self.selection.is_empty() {
            return self
                .issues
                .iter()
                .filter(|i| self.selection.contains(&i.id))
                .cloned()
                .collect();
        }
        self.selected_issue(cx).into_iter().collect()
    }

    fn update_issues(&mut self, ids: &[IssueId], change: IssueChange, cx: &mut Context<Self>) {
        if self
            .report(self.db.update_issues(ids, &change), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    fn apply_relation(
        &mut self,
        relation: Relation,
        ids: &[IssueId],
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.update_issues(ids, relation.change(name), cx);
    }

    /// Applies `relation` to the target issues, or opens a picker when no
    /// name is given.
    fn on_relation(
        &mut self,
        relation: Relation,
        name: Option<&String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return;
        }
        match name {
            Some(name) => {
                let ids: Vec<_> = issues.iter().map(|i| i.id).collect();
                self.apply_relation(relation, &ids, name.clone(), cx);
            }
            None => self.open_relation_picker(relation, &issues, window, cx),
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

    fn set_status(&mut self, ids: &[IssueId], status: Status, cx: &mut Context<Self>) {
        self.update_issues(ids, IssueChange::SetStatus(status), cx);
    }

    fn on_set_status(&mut self, action: &SetStatus, window: &mut Window, cx: &mut Context<Self>) {
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return;
        }
        match action.status {
            Some(status) => {
                let ids: Vec<_> = issues.iter().map(|i| i.id).collect();
                self.set_status(&ids, status, cx);
            }
            None => self.open_status_picker(&issues, window, cx),
        }
    }

    /// Edits one issue: the action's, else the single target issue.
    fn on_edit_issue(&mut self, action: &Edit, window: &mut Window, cx: &mut Context<Self>) {
        let issue = match action.id {
            Some(id) => self.report(self.db.get_issue(id), cx),
            None => match self.target_issues(cx).as_slice() {
                [] => None,
                [issue] => Some(issue.clone()),
                _ => {
                    self.error = Some("Select a single issue to edit".into());
                    cx.notify();
                    None
                }
            },
        };
        let Some(issue) = issue else {
            return;
        };
        if action.title.is_none() && action.description.is_none() {
            return self.show_issue(issue.id, window, cx);
        };
        let edit = IssueEdit {
            title: action.title.clone(),
            description: action.description.clone(),
        };
        if self
            .report(self.db.edit_issue(issue.id, &edit), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    /// Opens the menu of the target issues, at `position` (window
    /// coordinates) when opened with the mouse.
    fn show_issue_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issues = self.target_issues(cx);
        let Some(first) = issues.first() else {
            return;
        };
        let highlighted = self.selected.unwrap_or(first.id);
        let mut entries: Vec<MenuEntry> = Vec::new();
        if let [issue] = issues.as_slice() {
            if self.detail.is_none() {
                entries.push(("Open", Box::new(issue::Open { id: Some(issue.id) })));
            }
            let edit = Edit {
                id: Some(issue.id),
                ..Default::default()
            };
            entries.push(("Edit", Box::new(edit)));
        }
        entries.push(("Set Status…", Box::new(SetStatus::default())));
        entries.push(("Add Label…", Box::new(AddLabel::default())));
        if issues.iter().any(|i| !i.labels.is_empty()) {
            entries.push(("Remove Label…", Box::new(RemoveLabel::default())));
        }
        entries.push(("Add to Project…", Box::new(AddToProject::default())));
        if issues.iter().any(|i| !i.projects.is_empty()) {
            entries.push((
                "Remove from Project…",
                Box::new(RemoveFromProject::default()),
            ));
        }
        entries.push(("Archive", Box::new(issue::Archive)));
        entries.push((
            "Delete…",
            Box::new(issue::Delete {
                ids: issues.iter().map(|i| i.id).collect(),
                ..Default::default()
            }),
        ));
        let title = match issues.as_slice() {
            [issue] => issue.id.to_string(),
            _ => format!("{} issues", issues.len()),
        };
        // The entries act on the open issue, or the selection from the issue list.
        let focus = match &self.detail {
            Some(detail) => detail.focus_handle().clone(),
            None => self.list_focus.clone(),
        };
        window.focus(&focus);
        self.open_context_menu(
            &title,
            entries,
            MenuTarget::Issue(highlighted),
            position,
            window,
            cx,
        );
    }

    fn on_show_issue_menu(
        &mut self,
        _: &issue_list::ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_issue_menu(None, window, cx);
    }

    fn on_new_issue(&mut self, _: &issue::New, window: &mut Window, cx: &mut Context<Self>) {
        self.close_modal(window, cx);
        self.close_detail(cx);
        if self.showing_archive {
            self.close_archive(window, cx);
        }
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

    // ---- multiple selection -------------------------------------------------

    fn toggle_selected(
        &mut self,
        _: &issue_list::ToggleSelected,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected else {
            return;
        };
        if !self.selection.remove(&id) {
            self.selection.insert(id);
        }
        self.selection_anchor = Some(id);
        cx.notify();
    }

    fn select_all(&mut self, _: &issue_list::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = self.issues.iter().map(|i| i.id).collect();
        cx.notify();
    }

    fn clear_selection(
        &mut self,
        _: &issue_list::ClearSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            // Leave the key to other bindings.
            return cx.propagate();
        }
        self.selection.clear();
        self.selection_anchor = None;
        cx.notify();
    }

    fn extend_selection_down(
        &mut self,
        _: &issue_list::ExtendSelectionDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.selected_index().map_or(0, |ix| ix + 1);
        self.extend_selection_to(ix, cx);
    }

    fn extend_selection_up(
        &mut self,
        _: &issue_list::ExtendSelectionUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.selected_index().map_or(0, |ix| ix.saturating_sub(1));
        self.extend_selection_to(ix, cx);
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
        let in_selection = self.selection.contains(&issue_id);
        let menu_open = matches!(
            &self.modal,
            Some(OpenModal {
                modal: Modal::ContextMenu {
                    target: MenuTarget::Issue(id),
                    ..
                },
                ..
            }) if *id == issue_id
        );

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
                this.point_at(ix, window, cx);
                window.dispatch_action(SetStatus::default().boxed_clone(), cx);
            }));

        div()
            .id(("issue", issue_id.0 as u64))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_1()
            .map(|s| {
                let highlighted = selected && (list_focused || menu_open);
                let bg = match (in_selection, highlighted) {
                    (true, true) => Some(theme::SELECTION_HIGHLIGHT),
                    (true, false) => Some(theme::SELECTION),
                    (false, true) => Some(theme::HIGHLIGHT),
                    (false, false) => selected.then_some(theme::HOVER),
                };
                match bg {
                    Some(bg) => s.bg(rgb(bg)),
                    None => s,
                }
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                let modifiers = event.modifiers();
                window.focus(&this.list_focus);
                if modifiers.shift {
                    this.extend_selection_to(ix, cx);
                } else if modifiers.platform {
                    this.select_index(ix, cx);
                    window.dispatch_action(issue_list::ToggleSelected.boxed_clone(), cx);
                } else {
                    this.select_index(ix, cx);
                    window.dispatch_action(issue_list::ClearSelection.boxed_clone(), cx);
                    if event.click_count() >= 2 {
                        window.dispatch_action(issue::Open::default().boxed_clone(), cx);
                    }
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.point_at(ix, window, cx);
                    this.show_issue_menu(Some(event.position), window, cx);
                }),
            )
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
            .when(!issue.description.is_empty(), |row| {
                row.child(description_icon())
            })
            .children(
                issue
                    .checklist_progress()
                    .map(|(done, total)| checklist_badge(done, total)),
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
                    .map(|label| chip(label.clone(), self.label_chip_color(label))),
            )
    }

    /// Chip color of an issue's label.
    pub(super) fn label_chip_color(&self, name: &str) -> u32 {
        let color = self
            .labels
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(name))
            .and_then(|l| l.color);
        theme::label_color(color)
    }

    /// Highlights the issue at `ix` to act on it with the mouse: within the
    /// selection, the selection stays (and is acted on); elsewhere, it is
    /// cleared so that only this issue is.
    fn point_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .issues
            .get(ix)
            .is_some_and(|i| !self.selection.contains(&i.id))
        {
            self.selection.clear();
        }
        self.select_index(ix, cx);
        window.focus(&self.list_focus);
    }

    fn render_modal(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let modal = &self.modal.as_ref()?.modal;
        let picker = modal.picker().clone();
        let position = match modal {
            Modal::ContextMenu { position, .. } => *position,
            _ => None,
        };
        // Clicks inside the picker must not reach the backdrop.
        let content = div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(picker);
        let backdrop = div()
            .id("modal-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_modal(window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| {
                    this.close_modal(window, cx);
                }),
            );
        let backdrop = match position {
            Some(position) => backdrop.child(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(content),
            ),
            None => backdrop.flex().justify_center().pt(px(80.)).child(content),
        };
        Some(deferred(backdrop).with_priority(1))
    }
}

/// " (3 issues)" when acting on several issues, to append to a prompt.
fn of_issues(issues: &[Issue]) -> String {
    match issues.len() {
        0 | 1 => String::new(),
        n => format!(" ({n} issues)"),
    }
}

/// A small rounded tag, used for projects and labels.
/// Checklist progress of an issue in the list: done/total items.
/// Shown in the list for issues with a description: a few lines of text.
fn description_icon() -> impl IntoElement {
    let line = |width: f32| {
        div()
            .w(px(width))
            .h(px(1.5))
            .rounded_full()
            .bg(rgb(theme::TEXT_MUTED))
    };
    div()
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.5))
        .size(px(12.))
        .child(line(12.))
        .child(line(12.))
        .child(line(8.))
}

fn checklist_badge(done: usize, total: usize) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(12.))
        .text_color(rgb(if done == total {
            theme::ACCENT
        } else {
            theme::TEXT_MUTED
        }))
        .child(format!("✓ {done}/{total}"))
}

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
        self.sync_checklist(window, cx);
        let list_focused = self.list_focus.is_focused(window);
        div()
            .key_context("Workspace")
            .on_action(cx.listener(Self::toggle_command_palette))
            .on_action(cx.listener(Self::on_new_issue))
            .on_action(cx.listener(Self::on_set_status))
            .on_action(cx.listener(Self::on_edit_issue))
            .on_action(cx.listener(Self::on_open_issue))
            .on_action(cx.listener(Self::on_add_checklist_item))
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
            .on_action(cx.listener(Self::on_move_label))
            .on_action(cx.listener(Self::on_toggle_label_filter))
            .on_action(cx.listener(Self::on_exclude_label_filter))
            .on_action(cx.listener(Self::on_toggle_project_filter))
            .on_action(cx.listener(Self::on_toggle_status_filter))
            .on_action(cx.listener(Self::on_toggle_open_only))
            .on_action(cx.listener(Self::on_toggle_no_project))
            .on_action(cx.listener(Self::on_clear_filter))
            .on_action(cx.listener(Self::on_all_statuses))
            .on_action(cx.listener(Self::on_save_view))
            .on_action(cx.listener(Self::on_save_view_as))
            .on_action(cx.listener(Self::on_revert_view))
            .on_action(cx.listener(Self::on_rename_view))
            .on_action(cx.listener(Self::on_delete_view))
            .on_action(cx.listener(Self::on_rename_project))
            .on_action(cx.listener(Self::on_set_project_status))
            .on_action(cx.listener(Self::on_delete_project))
            .on_action(cx.listener(Self::on_rename_label))
            .on_action(cx.listener(Self::on_set_label_color))
            .on_action(cx.listener(Self::on_delete_label))
            .on_action(cx.listener(Self::on_focus_list))
            .on_action(cx.listener(Self::on_open_keymap))
            .on_action(cx.listener(Self::on_reload_keymap))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_focus_sidebar))
            .on_action(cx.listener(Self::on_open_archive))
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
                    .map(|s| {
                        if let Some(detail) = &self.detail {
                            s.child(self.render_detail(detail, cx).into_any_element())
                        } else if self.showing_archive {
                            s.child(self.render_archive(list_focused, cx).into_any_element())
                        } else {
                            s.child(self.render_main(list_focused, cx).into_any_element())
                        }
                    }),
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
                    .on_action(cx.listener(Self::toggle_selected))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::clear_selection))
                    .on_action(cx.listener(Self::extend_selection_down))
                    .on_action(cx.listener(Self::extend_selection_up))
                    .on_action(cx.listener(Self::on_archive_issue))
                    .on_action(cx.listener(Self::on_delete_issue))
                    .on_action(cx.listener(Self::on_show_issue_menu))
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
                                .child(if self.scope.filter == IssueFilter::default() {
                                    "No issues yet. Type a title above and press Enter."
                                } else if self.scope.filter == scope::Source::All.filter() {
                                    "No open issues. Type a title above and press Enter."
                                } else {
                                    "No issues here."
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
