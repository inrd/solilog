//! Renaming, deleting and changing the status of what the sidebar lists
//! (views, projects, labels), and the sidebar's context menu that offers it
//! (context menus in general: see also the archive's).
//!
//! Each action names its target, or acts on what the issue list shows. The
//! scope and the sidebar cursor follow renames, so the list keeps showing
//! the same thing (with any unsaved filter edits).

use gpui::{Action, AppContext as _, Context, Pixels, Point, Window, px};
use solilog_core::{IssueFilter, ProjectStatus, ProjectUpdate, label_matches, renamed_label};

use super::scope::Source;
use super::sidebar::SidebarItem;
use super::{CreateAction, MenuEntry, MenuTarget, Modal, Workspace};
use crate::ui::actions::{label, project, sidebar, view};
use crate::ui::picker::{Picker, PickerItem};
use crate::ui::status_icon::project_status_icon;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    View,
    Project,
    Label,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Kind::View => "view",
            Kind::Project => "project",
            Kind::Label => "label",
        }
    }

    /// `name` after renaming `old` to `new`, if the rename affects it (a
    /// label's sublabels move with it).
    fn renamed(self, name: &str, old: &str, new: &str) -> Option<String> {
        match self {
            Kind::View | Kind::Project => name.eq_ignore_ascii_case(old).then(|| new.to_owned()),
            Kind::Label => renamed_label(name, old, new),
        }
    }

    /// Whether deleting `deleted` deletes `name` too.
    fn deletes(self, name: &str, deleted: &str) -> bool {
        match self {
            Kind::View | Kind::Project => name.eq_ignore_ascii_case(deleted),
            Kind::Label => label_matches(name, deleted),
        }
    }

    fn rename_in_filter(self, filter: &mut IssueFilter, old: &str, new: &str) {
        match self {
            Kind::View => {}
            Kind::Project => filter.rename_project(old, new),
            Kind::Label => filter.rename_label(old, new),
        }
    }
}

/// The name in a scope source or sidebar item of the given kind.
fn source_name(kind: Kind, source: &mut Source) -> Option<&mut String> {
    match (kind, source) {
        (Kind::View, Source::View(view)) => Some(&mut view.name),
        (Kind::Project, Source::Project(name)) | (Kind::Label, Source::Label(name)) => Some(name),
        _ => None,
    }
}

fn item_name(kind: Kind, item: &mut SidebarItem) -> Option<&mut String> {
    match (kind, item) {
        (Kind::View, SidebarItem::View(name))
        | (Kind::Project, SidebarItem::Project(name))
        | (Kind::Label, SidebarItem::Label(name)) => Some(name),
        _ => None,
    }
}

/// The context menu of a sidebar row; empty for rows without one.
fn menu_entries(item: &SidebarItem) -> Vec<MenuEntry> {
    let open: MenuEntry = ("Open", item.action());
    match item {
        SidebarItem::View(name) => {
            let name = Some(name.clone());
            vec![
                open,
                (
                    "Rename…",
                    Box::new(view::Rename {
                        name: name.clone(),
                        new_name: None,
                    }),
                ),
                (
                    "Delete…",
                    Box::new(view::Delete {
                        name,
                        confirmed: false,
                    }),
                ),
            ]
        }
        SidebarItem::Project(name) => {
            let name = Some(name.clone());
            vec![
                open,
                (
                    "Rename…",
                    Box::new(project::Rename {
                        name: name.clone(),
                        new_name: None,
                    }),
                ),
                (
                    "Set Status…",
                    Box::new(project::SetStatus {
                        name: name.clone(),
                        status: None,
                    }),
                ),
                (
                    "Archive",
                    Box::new(project::SetStatus {
                        name: name.clone(),
                        status: Some(ProjectStatus::Archived),
                    }),
                ),
                (
                    "Delete…",
                    Box::new(project::Delete {
                        name,
                        confirmed: false,
                    }),
                ),
            ]
        }
        SidebarItem::Label(name) => {
            let name = Some(name.clone());
            vec![
                open,
                (
                    "Rename…",
                    Box::new(label::Rename {
                        name: name.clone(),
                        new_name: None,
                    }),
                ),
                (
                    "Delete…",
                    Box::new(label::Delete {
                        name,
                        confirmed: false,
                    }),
                ),
            ]
        }
        SidebarItem::AllIssues | SidebarItem::ClosedProjects => Vec::new(),
    }
}

impl Workspace {
    // ---- context menu -----------------------------------------------------

    /// Opens the menu of the highlighted sidebar row, at `position` (window
    /// coordinates) when opened with the mouse.
    pub(super) fn show_context_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item = self.sidebar_cursor.clone();
        let title = match &item {
            SidebarItem::View(name) | SidebarItem::Project(name) | SidebarItem::Label(name) => {
                name.clone()
            }
            SidebarItem::AllIssues | SidebarItem::ClosedProjects => String::new(),
        };
        let entries = menu_entries(&item);
        self.open_context_menu(
            &title,
            entries,
            MenuTarget::Sidebar(item),
            position,
            window,
            cx,
        );
    }

    /// Opens a menu whose entries each dispatch an action; does nothing
    /// without entries.
    pub(super) fn open_context_menu(
        &mut self,
        title: &str,
        entries: Vec<MenuEntry>,
        target: MenuTarget,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (labels, actions): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        if actions.is_empty() {
            return;
        }
        let items = labels.into_iter().map(PickerItem::new).collect();
        let picker = cx.new(|cx| Picker::new(title, items, 0, cx).width(px(240.)));
        self.open_modal(
            picker,
            |picker| Modal::ContextMenu {
                picker,
                actions,
                target,
                position,
            },
            window,
            cx,
        );
    }

    pub(super) fn on_show_context_menu(
        &mut self,
        _: &sidebar::ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_context_menu(None, window, cx);
    }

    // ---- shared steps -------------------------------------------------------

    /// The stored name of the `kind` named `name`, or of the one being shown.
    fn target(
        &mut self,
        kind: Kind,
        name: Option<&String>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let found = match name {
            Some(name) => match kind {
                Kind::View => self.db.get_view(name).map(|v| v.name),
                Kind::Project => self.db.get_project(name).map(|p| p.name),
                Kind::Label => self.db.get_label(name).map(|l| l.name),
            },
            None => match (kind, &self.scope.source) {
                (Kind::View, Source::View(view)) => Ok(view.name.clone()),
                (Kind::Project, Source::Project(name)) | (Kind::Label, Source::Label(name)) => {
                    Ok(name.clone())
                }
                _ => Err(anyhow::anyhow!("Not showing a {}", kind.noun())),
            },
        };
        self.report(found, cx)
    }

    /// Asks for a new name, prefilled with the current one; `rename` builds
    /// the action that applies it.
    fn ask_new_name(
        &mut self,
        kind: Kind,
        name: &str,
        rename: impl Fn(String) -> Box<dyn Action> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = format!("Rename {} “{name}” to…", kind.noun());
        let picker = cx.new(|cx| {
            Picker::new(&placeholder, Vec::new(), 0, cx)
                .creatable("Rename to")
                .query(name, cx)
        });
        let create: CreateAction = Box::new(rename);
        self.open_modal(
            picker,
            |picker| Modal::ActionPicker {
                picker,
                actions: Vec::new(),
                create: Some(create),
            },
            window,
            cx,
        );
    }

    /// Makes the scope and sidebar cursor follow a rename, then reloads.
    fn follow_rename(&mut self, kind: Kind, old: &str, new: &str, cx: &mut Context<Self>) {
        kind.rename_in_filter(&mut self.scope.filter, old, new);
        if let Source::View(view) = &mut self.scope.source {
            kind.rename_in_filter(&mut view.filter, old, new);
        }
        for name in [
            source_name(kind, &mut self.scope.source),
            item_name(kind, &mut self.sidebar_cursor),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(renamed) = kind.renamed(name, old, new) {
                *name = renamed;
            }
        }
        self.reload(cx);
    }

    /// Asks before deleting; confirming dispatches `delete`.
    fn confirm_delete(
        &mut self,
        kind: Kind,
        name: &str,
        hint: Option<String>,
        delete: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let noun = kind.noun();
        let item = PickerItem::new(format!("Delete {noun} “{name}”")).hint(hint);
        self.open_action_picker(
            &format!("Delete {noun} “{name}”?"),
            vec![(item, delete)],
            0,
            window,
            cx,
        );
    }

    /// Leaves what was deleted: shows all issues if it was being shown (the
    /// archive stays open if it is), and drops it from the filter otherwise.
    fn follow_delete(&mut self, kind: Kind, name: &str, cx: &mut Context<Self>) {
        let shown =
            source_name(kind, &mut self.scope.source).is_some_and(|n| kind.deletes(n, name));
        if shown && self.showing_archive {
            // Deleted from the archive: stay there.
            self.scope = Default::default();
        } else if shown {
            return self.show(Ok(Source::All), cx);
        }
        match kind {
            Kind::View => {}
            Kind::Project => self.scope.filter.drop_project(name),
            Kind::Label => self.scope.filter.drop_label(name),
        }
        self.reload(cx);
    }

    // ---- views ----------------------------------------------------------------

    pub(super) fn on_rename_view(
        &mut self,
        action: &view::Rename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::View, action.name.as_ref(), cx) else {
            return;
        };
        let Some(new_name) = &action.new_name else {
            let target = name.clone();
            let rename = move |new_name| -> Box<dyn Action> {
                Box::new(view::Rename {
                    name: Some(target.clone()),
                    new_name: Some(new_name),
                })
            };
            return self.ask_new_name(Kind::View, &name, rename, window, cx);
        };
        if let Some(view) = self.report(self.db.rename_view(&name, new_name), cx) {
            self.follow_rename(Kind::View, &name, &view.name, cx);
        }
    }

    pub(super) fn on_delete_view(
        &mut self,
        action: &view::Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::View, action.name.as_ref(), cx) else {
            return;
        };
        if !action.confirmed {
            let delete = Box::new(view::Delete {
                name: Some(name.clone()),
                confirmed: true,
            });
            return self.confirm_delete(Kind::View, &name, None, delete, window, cx);
        }
        if self.report(self.db.delete_view(&name), cx).is_some() {
            self.follow_delete(Kind::View, &name, cx);
        }
    }

    // ---- projects -------------------------------------------------------------

    pub(super) fn on_rename_project(
        &mut self,
        action: &project::Rename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::Project, action.name.as_ref(), cx) else {
            return;
        };
        let Some(new_name) = &action.new_name else {
            let target = name.clone();
            let rename = move |new_name| -> Box<dyn Action> {
                Box::new(project::Rename {
                    name: Some(target.clone()),
                    new_name: Some(new_name),
                })
            };
            return self.ask_new_name(Kind::Project, &name, rename, window, cx);
        };
        let update = ProjectUpdate {
            name: Some(new_name.clone()),
            ..Default::default()
        };
        if let Some(project) = self.report(self.db.update_project(&name, &update), cx) {
            self.follow_rename(Kind::Project, &name, &project.name, cx);
        }
    }

    pub(super) fn on_set_project_status(
        &mut self,
        action: &project::SetStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::Project, action.name.as_ref(), cx) else {
            return;
        };
        let Some(status) = action.status else {
            let current = self
                .projects
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.status);
            let highlighted = ProjectStatus::ALL
                .iter()
                .position(|s| Some(*s) == current)
                .unwrap_or(0);
            let entries = ProjectStatus::ALL
                .into_iter()
                .map(|status| {
                    let action = project::SetStatus {
                        name: Some(name.clone()),
                        status: Some(status),
                    };
                    (
                        PickerItem::new(status.label()).icon(move || project_status_icon(status)),
                        Box::new(action) as Box<dyn Action>,
                    )
                })
                .collect();
            let placeholder = format!("Set status of “{name}”…");
            return self.open_action_picker(&placeholder, entries, highlighted, window, cx);
        };
        let update = ProjectUpdate {
            status: Some(status),
            ..Default::default()
        };
        if self
            .report(self.db.update_project(&name, &update), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    pub(super) fn on_delete_project(
        &mut self,
        action: &project::Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::Project, action.name.as_ref(), cx) else {
            return;
        };
        if !action.confirmed {
            let delete = Box::new(project::Delete {
                name: Some(name.clone()),
                confirmed: true,
            });
            let hint = Some("issues are kept".to_string());
            return self.confirm_delete(Kind::Project, &name, hint, delete, window, cx);
        }
        if self.report(self.db.delete_project(&name), cx).is_some() {
            self.follow_delete(Kind::Project, &name, cx);
        }
    }

    // ---- labels ---------------------------------------------------------------

    pub(super) fn on_rename_label(
        &mut self,
        action: &label::Rename,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::Label, action.name.as_ref(), cx) else {
            return;
        };
        let Some(new_name) = &action.new_name else {
            let target = name.clone();
            let rename = move |new_name| -> Box<dyn Action> {
                Box::new(label::Rename {
                    name: Some(target.clone()),
                    new_name: Some(new_name),
                })
            };
            return self.ask_new_name(Kind::Label, &name, rename, window, cx);
        };
        if let Some(label) = self.report(self.db.rename_label(&name, new_name), cx) {
            self.follow_rename(Kind::Label, &name, &label.name, cx);
        }
    }

    pub(super) fn on_delete_label(
        &mut self,
        action: &label::Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.target(Kind::Label, action.name.as_ref(), cx) else {
            return;
        };
        if !action.confirmed {
            let sublabels = self
                .labels
                .iter()
                .filter(|l| label_matches(&l.name, &name))
                .count()
                .saturating_sub(1);
            let issues = self
                .labels
                .iter()
                .find(|l| l.name == name)
                .map_or(0, |l| l.issue_count);
            let mut hint = Vec::new();
            if sublabels > 0 {
                hint.push(plural(sublabels, "sublabel"));
            }
            if issues > 0 {
                hint.push(format!("on {}", plural(issues as usize, "issue")));
            }
            let delete = Box::new(label::Delete {
                name: Some(name.clone()),
                confirmed: true,
            });
            let hint = (!hint.is_empty()).then(|| format!("with {}", hint.join(", ")));
            return self.confirm_delete(Kind::Label, &name, hint, delete, window, cx);
        }
        if self.report(self.db.delete_label(&name), cx).is_some() {
            self.follow_delete(Kind::Label, &name, cx);
        }
    }
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
