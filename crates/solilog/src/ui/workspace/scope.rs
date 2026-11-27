//! What the issue list shows: all issues, a saved view, a project or a label,
//! and the filter in effect.

use gpui::{Action, AppContext as _, Context, Window};
use solilog_core::{IssueFilter, NewIssue, ProjectStatus, View};

use super::{CreateAction, Modal, Workspace};
use crate::ui::actions::view;
use crate::ui::picker::{Picker, PickerItem};

/// Where the shown issues come from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    All,
    /// Issues with unreviewed non-user changes (open or closed).
    Unreviewed,
    /// A saved view, as stored.
    View(View),
    Project(String),
    Label(String),
}

impl Source {
    /// The filter this source stands for. Apart from saved views and
    /// unreviewed, done and cancelled issues are hidden at first.
    pub fn filter(&self) -> IssueFilter {
        match self {
            Source::All => IssueFilter {
                open_only: true,
                ..Default::default()
            },
            Source::Unreviewed => IssueFilter {
                unreviewed: true,
                ..Default::default()
            },
            Source::View(view) => view.filter.clone(),
            Source::Project(name) => IssueFilter {
                projects: vec![name.clone()],
                open_only: true,
                ..Default::default()
            },
            Source::Label(name) => IssueFilter {
                labels: vec![name.clone()],
                open_only: true,
                ..Default::default()
            },
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Source::All => "All issues",
            Source::Unreviewed => "Unreviewed",
            Source::View(view) => &view.name,
            Source::Project(name) | Source::Label(name) => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub source: Source,
    /// The filter in effect.
    pub filter: IssueFilter,
}

impl Default for Scope {
    fn default() -> Self {
        Self::new(Source::All)
    }
}

impl Scope {
    pub fn new(source: Source) -> Self {
        Self {
            filter: source.filter(),
            source,
        }
    }
}

impl Workspace {
    /// Re-reads the scope's source from the database; an edited filter is
    /// kept. Returns an error message if the source no longer exists, after
    /// falling back to all issues.
    pub(super) fn refresh_scope(&mut self) -> Option<String> {
        let found = match &self.scope.source {
            Source::All | Source::Unreviewed => return None,
            Source::View(view) => self.db.get_view(&view.name).map(Source::View),
            Source::Project(name) => self.db.get_project(name).map(|p| Source::Project(p.name)),
            Source::Label(name) => self.db.get_label(name).map(|l| Source::Label(l.name)),
        };
        match found {
            Ok(source) => {
                let edited = self.scope.filter != self.scope.source.filter();
                self.scope = Scope {
                    filter: if edited {
                        self.scope.filter.clone()
                    } else {
                        source.filter()
                    },
                    source,
                };
                None
            }
            Err(err) => {
                self.scope = Scope::default();
                Some(format!("{err:#}"))
            }
        }
    }

    /// A new issue created in a scope gets the scope's labels, and its project
    /// if it names exactly one.
    pub(super) fn new_issue(&self, title: String) -> NewIssue {
        let filter = &self.scope.filter;
        let mut new = NewIssue::new(title);
        new.labels = filter.labels.clone();
        if let [project] = filter.projects.as_slice() {
            new.projects = vec![project.clone()];
        }
        new
    }

    pub(super) fn show(&mut self, source: anyhow::Result<Source>, cx: &mut Context<Self>) {
        let Some(source) = self.report(source, cx) else {
            return;
        };
        self.close_detail(cx);
        // Before the project header shows another project's description.
        self.save_project_description(cx);
        self.scope = Scope::new(source);
        self.showing_archive = false;
        self.list.selected = None;
        self.list.selection.clear();
        self.reload(cx);
        self.select_index(0, cx);
        self.sidebar.cursor = self.active_sidebar_item();
    }

    /// Opens a picker whose items each dispatch an action.
    pub(super) fn open_action_picker(
        &mut self,
        placeholder: &str,
        entries: Vec<(PickerItem, Box<dyn Action>)>,
        highlighted: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_creatable_action_picker(placeholder, entries, highlighted, None, window, cx);
    }

    /// Like [`Self::open_action_picker`], plus a "`label` “text”" row whose
    /// action `create` builds from the typed text.
    pub(super) fn open_creatable_action_picker(
        &mut self,
        placeholder: &str,
        entries: Vec<(PickerItem, Box<dyn Action>)>,
        highlighted: usize,
        create: Option<(&'static str, CreateAction)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (items, actions): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        let create_label = create.as_ref().map(|(label, _)| *label);
        let picker = cx.new(|cx| {
            let picker = Picker::new(placeholder, items, highlighted, cx);
            match create_label {
                Some(label) => picker.creatable(label),
                None => picker,
            }
        });
        let create = create.map(|(_, build)| build);
        self.open_modal(
            picker,
            |picker| Modal::ActionPicker {
                picker,
                actions,
                create,
            },
            window,
            cx,
        );
    }

    fn save_view(&mut self, name: &str, cx: &mut Context<Self>) {
        let saved = self.db.save_view(name, &self.scope.filter);
        if let Some(view) = self.report(saved, cx) {
            self.scope = Scope::new(Source::View(view));
            self.reload(cx);
            self.sidebar.cursor = self.active_sidebar_item();
        }
    }

    pub(super) fn on_save_view(
        &mut self,
        action: &view::Save,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (&action.name, &self.scope.source) {
            (Some(name), _) => self.save_view(&name.clone(), cx),
            (None, Source::View(view)) => self.save_view(&view.name.clone(), cx),
            (None, _) => self.on_save_view_as(&view::SaveAs, window, cx),
        }
    }

    pub(super) fn on_save_view_as(
        &mut self,
        _: &view::SaveAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = self
            .views
            .iter()
            .map(|v| {
                let item = PickerItem::new(v.name.clone()).hint(Some("replace"));
                let action = view::Save {
                    name: Some(v.name.clone()),
                };
                (item, Box::new(action) as Box<dyn Action>)
            })
            .collect();
        let create: CreateAction = Box::new(|name| Box::new(view::Save { name: Some(name) }));
        self.open_creatable_action_picker(
            "Save view as…",
            entries,
            0,
            Some(("New view", create)),
            window,
            cx,
        );
    }

    pub(super) fn on_revert_view(
        &mut self,
        _: &view::Revert,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scope.filter = self.scope.source.filter();
        self.footer.error = None;
        self.reload(cx);
    }

    pub(super) fn on_show_all(
        &mut self,
        _: &view::ShowAll,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show(Ok(Source::All), cx);
    }

    pub(super) fn on_show_unreviewed(
        &mut self,
        _: &view::ShowUnreviewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show(Ok(Source::Unreviewed), cx);
    }

    pub(super) fn on_open_view(
        &mut self,
        action: &view::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = &action.name {
            let view = self.db.get_view(name).map(Source::View);
            return self.show(view, cx);
        }
        let mut entries: Vec<(PickerItem, Box<dyn Action>)> = vec![
            (PickerItem::new("All issues"), Box::new(view::ShowAll)),
            (
                PickerItem::new("Unreviewed"),
                Box::new(view::ShowUnreviewed),
            ),
        ];
        entries.extend(self.views.iter().map(|v| {
            let item = PickerItem::new(v.name.clone()).hint(Some(v.filter.to_string()));
            let action = view::Open {
                name: Some(v.name.clone()),
            };
            (item, Box::new(action) as Box<dyn Action>)
        }));
        let current = match &self.scope.source {
            Source::Unreviewed => Some(1),
            Source::View(v) => self
                .views
                .iter()
                .position(|w| w.name == v.name)
                .map(|i| i + 2),
            _ => None,
        };
        self.open_action_picker("Open view…", entries, current.unwrap_or(0), window, cx);
    }

    pub(super) fn on_open_project(
        &mut self,
        action: &view::OpenProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = &action.name {
            let project = self.db.get_project(name).map(|p| Source::Project(p.name));
            return self.show(project, cx);
        }
        // Archived projects are only listed in the archive.
        let entries = self
            .projects
            .iter()
            .filter(|p| p.status != ProjectStatus::Archived)
            .map(|p| {
                let hint = p.status.is_closed().then(|| p.status.label());
                let action = view::OpenProject {
                    name: Some(p.name.clone()),
                };
                (
                    PickerItem::new(p.name.clone()).hint(hint),
                    Box::new(action) as Box<dyn Action>,
                )
            })
            .collect();
        self.open_action_picker("Open project…", entries, 0, window, cx);
    }

    pub(super) fn on_open_label(
        &mut self,
        action: &view::OpenLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = &action.name {
            let label = self.db.get_label(name).map(|l| Source::Label(l.name));
            return self.show(label, cx);
        }
        let entries = self
            .labels
            .iter()
            .map(|l| {
                let action = view::OpenLabel {
                    name: Some(l.name.clone()),
                };
                (
                    PickerItem::new(l.name.clone()),
                    Box::new(action) as Box<dyn Action>,
                )
            })
            .collect();
        self.open_action_picker("Open label…", entries, 0, window, cx);
    }
}
