//! The sidebar: everything the issue list can show (all issues, saved views,
//! projects, labels), with the number of open issues in each. Done projects
//! sit in a collapsed section; archived ones are only in the archive.
//!
//! Key context: `Sidebar`. Each row stands for an action (e.g. `view::Open`);
//! confirming a row, by key or click, dispatches it. Projects and labels can
//! be dragged to reorder them: projects within their section (open or
//! closed), labels among their siblings. A label's sublabels can be
//! collapsed (`label::ToggleCollapsed`). Views, projects and labels have a
//! context menu (right click, or `sidebar::ShowContextMenu`).

use gpui::{
    Action, ClickEvent, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent,
    SharedString, Window, div, prelude::*, px,
};
use solilog_core::{IssueFilter, LABEL_SEPARATOR, Placement, ProjectStatus};

use super::scope::Source;
use super::{MenuTarget, Modal, OpenModal, Workspace};
use crate::ui::actions::{label, project, sidebar, view, workspace};
use crate::ui::assets;
use crate::ui::status_icon::project_status_icon;
use crate::ui::theme;
use crate::ui::tooltip::truncated_text;

pub const CONTEXT: &str = "Sidebar";
const WIDTH: f32 = 220.;

/// A selectable sidebar row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItem {
    AllIssues,
    Unreviewed,
    View(String),
    Project(String),
    /// Expands or collapses the done projects.
    ClosedProjects,
    Label(String),
}

impl SidebarItem {
    /// What confirming the row does.
    pub(super) fn action(&self) -> Box<dyn Action> {
        match self {
            SidebarItem::AllIssues => Box::new(view::ShowAll),
            SidebarItem::Unreviewed => Box::new(view::ShowUnreviewed),
            SidebarItem::View(name) => Box::new(view::Open {
                name: Some(name.clone()),
            }),
            SidebarItem::Project(name) => Box::new(view::OpenProject {
                name: Some(name.clone()),
            }),
            SidebarItem::ClosedProjects => Box::new(sidebar::ToggleClosedProjects),
            SidebarItem::Label(name) => Box::new(view::OpenLabel {
                name: Some(name.clone()),
            }),
        }
    }
}

/// Rows can only be reordered among rows of the same group.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Group {
    /// The open or the done projects.
    Projects { closed: bool },
    /// The children of a label (lowercase), or the top-level labels.
    Labels { parent: Option<String> },
}

/// A project or label row that can be reordered; the payload of a drag, which
/// also renders the row that follows the mouse.
#[derive(Clone)]
struct DraggedRow {
    name: String,
    title: SharedString,
    group: Group,
    /// Index among the rows of its kind, to tell moving up from moving down.
    index: usize,
}

impl DraggedRow {
    /// The action that moves this row next to `anchor`.
    fn move_action(&self, placement: Placement) -> Box<dyn Action> {
        let name = self.name.clone();
        let (before, after) = match placement {
            Placement::Before(anchor) => (Some(anchor), None),
            Placement::After(anchor) => (None, Some(anchor)),
        };
        match self.group {
            Group::Projects { .. } => Box::new(project::Move {
                name,
                before,
                after,
            }),
            Group::Labels { .. } => Box::new(label::Move {
                name,
                before,
                after,
            }),
        }
    }
}

impl Render for DraggedRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .w(px(WIDTH - 8.))
            .px_2()
            .py(px(2.))
            .rounded(t.radius_sm())
            .bg(t.colors.highlight)
            .text_size(px(14.))
            .line_height(px(20.))
            .text_color(t.colors.highlight_text)
            .truncate()
            .child(self.title.clone())
    }
}

/// Dot color before a sidebar row, resolved against the active theme when rendering.
enum RowDot {
    Accent,
    Label(solilog_core::LabelColor),
}

/// A rendered row: a section heading or an item.
enum Row {
    /// A section heading, with a button on the right that runs the action.
    Heading(&'static str, Option<Box<dyn Action>>),
    Item {
        item: SidebarItem,
        label: SharedString,
        /// Nesting level, for the label tree.
        depth: usize,
        count: usize,
        /// Overrides `count` on the right (e.g. the closed-projects chevron).
        trailing: Option<&'static str>,
        /// Shown as an indicator before the label.
        status: Option<ProjectStatus>,
        /// A label's color, shown as a dot before it.
        color: Option<RowDot>,
        /// For labels: whether its sublabels are hidden, `None` without any.
        collapsed: Option<bool>,
    },
}

impl Workspace {
    /// Number of open issues matching `filter` (names must be canonical).
    fn open_count(&self, filter: &IssueFilter) -> usize {
        self.all_issues
            .iter()
            .filter(|i| !i.status.is_closed() && filter.matches(i))
            .count()
    }

    fn sidebar_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Item {
            item: SidebarItem::AllIssues,
            label: "All issues".into(),
            depth: 0,
            count: self.open_count(&IssueFilter::default()),
            trailing: None,
            status: None,
            color: None,
            collapsed: None,
        }];
        let unreviewed_count = self.all_issues.iter().filter(|i| i.unreviewed).count();
        rows.push(Row::Item {
            item: SidebarItem::Unreviewed,
            label: "Unreviewed".into(),
            depth: 0,
            count: unreviewed_count,
            trailing: None,
            status: None,
            color: (unreviewed_count > 0).then_some(RowDot::Accent),
            collapsed: None,
        });
        if !self.views.is_empty() {
            rows.push(Row::Heading("Views", None));
            rows.extend(self.views.iter().map(|v| Row::Item {
                item: SidebarItem::View(v.name.clone()),
                label: v.name.clone().into(),
                depth: 0,
                count: self.open_count(&v.filter),
                trailing: None,
                status: None,
                color: None,
                collapsed: None,
            }));
        }

        // Always there, for its button that creates a project.
        rows.push(Row::Heading(
            "Projects",
            Some(Box::new(project::Create::default())),
        ));
        let project_row = |p: &solilog_core::Project| Row::Item {
            item: SidebarItem::Project(p.name.clone()),
            label: p.name.clone().into(),
            depth: 0,
            count: self.open_count(&Source::Project(p.name.clone()).filter()),
            trailing: None,
            status: Some(p.status),
            color: None,
            collapsed: None,
        };
        let (closed, open): (Vec<_>, Vec<_>) =
            self.sidebar_projects().partition(|p| p.status.is_closed());
        rows.extend(open.into_iter().map(project_row));
        if !closed.is_empty() {
            rows.push(Row::Item {
                item: SidebarItem::ClosedProjects,
                label: format!("Done ({})", closed.len()).into(),
                depth: 0,
                count: 0,
                trailing: Some(if self.show_closed_projects {
                    "▾"
                } else {
                    "▸"
                }),
                status: None,
                color: None,
                collapsed: None,
            });
            if self.show_closed_projects {
                rows.extend(closed.into_iter().map(|p| match project_row(p) {
                    Row::Item {
                        item,
                        label,
                        count,
                        status,
                        ..
                    } => Row::Item {
                        item,
                        label,
                        depth: 1,
                        count,
                        trailing: None,
                        status,
                        color: None,
                        collapsed: None,
                    },
                    heading => heading,
                }));
            }
        }

        if !self.labels.is_empty() {
            rows.push(Row::Heading("Labels", None));
            let labels = self.labels.iter().enumerate();
            rows.extend(
                labels
                    .filter(|(_, l)| !self.label_hidden(l))
                    .map(|(ix, l)| {
                        let leaf = l.name.rsplit(LABEL_SEPARATOR).next();
                        // Sublabels directly follow their parent.
                        let has_children = self.labels.get(ix + 1).is_some_and(|next| {
                            next.parent()
                                .is_some_and(|p| p.eq_ignore_ascii_case(&l.name))
                        });
                        Row::Item {
                            item: SidebarItem::Label(l.name.clone()),
                            label: leaf.unwrap_or(&l.name).to_string().into(),
                            depth: l.depth(),
                            count: self.open_count(&Source::Label(l.name.clone()).filter()),
                            trailing: None,
                            status: None,
                            color: l.display_color.map(RowDot::Label),
                            collapsed: has_children.then_some(l.collapsed),
                        }
                    }),
            );
        }
        rows
    }

    /// Whether a label is inside a collapsed one.
    fn label_hidden(&self, label: &solilog_core::Label) -> bool {
        self.labels.iter().any(|l| {
            l.collapsed
                && l.name.len() < label.name.len()
                && solilog_core::label_matches(&label.name, &l.name)
        })
    }

    pub(super) fn on_toggle_label_collapsed(
        &mut self,
        action: &label::ToggleCollapsed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = match (&action.name, &self.sidebar_cursor) {
            (Some(name), _) => name.clone(),
            (None, SidebarItem::Label(name)) if self.sidebar_focus.is_focused(window) => {
                name.clone()
            }
            _ => match &self.scope.source {
                Source::Label(name) => name.clone(),
                _ => {
                    self.report::<()>(Err(anyhow::anyhow!("Not showing a label")), cx);
                    return;
                }
            },
        };
        let collapsed = self
            .labels
            .iter()
            .any(|l| l.collapsed && l.name.eq_ignore_ascii_case(&name));
        let Some(label) = self.report(self.db.set_label_collapsed(&name, !collapsed), cx) else {
            return;
        };
        // The cursor can't stay on a row that is now hidden.
        if let SidebarItem::Label(cursor) = &self.sidebar_cursor
            && label.collapsed
            && solilog_core::label_matches(cursor, &label.name)
        {
            self.sidebar_cursor = SidebarItem::Label(label.name);
        }
        self.reload(cx);
    }

    /// The selectable items, in display order.
    pub(super) fn sidebar_items(&self) -> Vec<SidebarItem> {
        self.sidebar_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Item { item, .. } => Some(item),
                Row::Heading(..) => None,
            })
            .collect()
    }

    /// The item matching what the issue list currently shows.
    pub(super) fn active_sidebar_item(&self) -> SidebarItem {
        match &self.scope.source {
            Source::All => SidebarItem::AllIssues,
            Source::Unreviewed => SidebarItem::Unreviewed,
            Source::View(view) => SidebarItem::View(view.name.clone()),
            Source::Project(name) => SidebarItem::Project(name.clone()),
            Source::Label(name) => SidebarItem::Label(name.clone()),
        }
    }

    fn move_sidebar_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.sidebar_items();
        let current = items
            .iter()
            .position(|i| *i == self.sidebar_cursor)
            .unwrap_or(0);
        let ix = (current as isize + delta).clamp(0, items.len() as isize - 1) as usize;
        self.sidebar_cursor = items[ix].clone();
        cx.notify();
    }

    pub(super) fn on_toggle_sidebar(
        &mut self,
        _: &workspace::ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible && self.sidebar_focus.contains_focused(window, cx) {
            window.focus(&self.list_focus);
        }
        cx.notify();
    }

    pub(super) fn on_focus_sidebar(
        &mut self,
        _: &workspace::FocusSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = true;
        self.sidebar_cursor = self.active_sidebar_item();
        window.focus(&self.sidebar_focus);
        cx.notify();
    }

    fn on_sidebar_next(&mut self, _: &sidebar::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_sidebar_cursor(1, cx);
    }

    fn on_sidebar_previous(
        &mut self,
        _: &sidebar::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_sidebar_cursor(-1, cx);
    }

    pub(super) fn on_toggle_closed_projects(
        &mut self,
        _: &sidebar::ToggleClosedProjects,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_closed_projects = !self.show_closed_projects;
        cx.notify();
    }

    /// The projects the sidebar lists: all but archived ones.
    fn sidebar_projects(&self) -> impl Iterator<Item = &solilog_core::Project> {
        self.projects
            .iter()
            .filter(|p| p.status != ProjectStatus::Archived)
    }

    /// The reorderable row for a sidebar item, if it is a project or label.
    fn reorderable(&self, item: &SidebarItem) -> Option<DraggedRow> {
        match item {
            SidebarItem::Project(name) => {
                let index = self.sidebar_projects().position(|p| p.name == *name)?;
                let closed = self
                    .projects
                    .iter()
                    .find(|p| p.name == *name)?
                    .status
                    .is_closed();
                Some(DraggedRow {
                    name: name.clone(),
                    title: name.clone().into(),
                    group: Group::Projects { closed },
                    index,
                })
            }
            SidebarItem::Label(name) => {
                let index = self.labels.iter().position(|l| l.name == *name)?;
                let label = &self.labels[index];
                let leaf = name.rsplit(LABEL_SEPARATOR).next().unwrap_or(name);
                Some(DraggedRow {
                    name: name.clone(),
                    title: leaf.to_string().into(),
                    group: Group::Labels {
                        parent: label.parent().map(str::to_ascii_lowercase),
                    },
                    index,
                })
            }
            _ => None,
        }
    }

    /// The names of the rows in a group, in order.
    fn group_members(&self, group: &Group) -> Vec<&str> {
        match group {
            Group::Projects { closed } => self
                .sidebar_projects()
                .filter(|p| p.status.is_closed() == *closed)
                .map(|p| p.name.as_str())
                .collect(),
            Group::Labels { parent } => self
                .labels
                .iter()
                .filter(|l| l.parent().map(str::to_ascii_lowercase) == *parent)
                .map(|l| l.name.as_str())
                .collect(),
        }
    }

    /// Moves the highlighted project or label past its neighbor in its group.
    fn move_highlighted(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.reorderable(&self.sidebar_cursor) else {
            return;
        };
        let members = self.group_members(&row.group);
        let Some(ix) = members.iter().position(|name| *name == row.name) else {
            return;
        };
        let neighbor = if down {
            members.get(ix + 1)
        } else {
            ix.checked_sub(1).and_then(|ix| members.get(ix))
        };
        if let Some(neighbor) = neighbor {
            let neighbor = neighbor.to_string();
            let placement = if down {
                Placement::After(neighbor)
            } else {
                Placement::Before(neighbor)
            };
            window.dispatch_action(row.move_action(placement), cx);
        }
    }

    fn on_move_up(&mut self, _: &sidebar::MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_highlighted(false, window, cx);
    }

    fn on_move_down(&mut self, _: &sidebar::MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_highlighted(true, window, cx);
    }

    pub(super) fn on_move_project(
        &mut self,
        action: &project::Move,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let moved = Placement::from_options(action.before.clone(), action.after.clone())
            .and_then(|placement| self.db.move_project(&action.name, &placement));
        if self.report(moved, cx).is_some() {
            self.reload(cx);
        }
    }

    pub(super) fn on_move_label(
        &mut self,
        action: &label::Move,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let moved = Placement::from_options(action.before.clone(), action.after.clone())
            .and_then(|placement| self.db.move_label(&action.name, &placement));
        if self.report(moved, cx).is_some() {
            self.reload(cx);
        }
    }

    fn on_sidebar_open(&mut self, _: &sidebar::Open, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(self.sidebar_cursor.action(), cx);
    }

    /// Makes a project or label row draggable, and a drop target for the
    /// rows of its group. The dropped row takes the target's place: it goes
    /// after the target when moving down, before it when moving up.
    fn drag_and_drop(
        &self,
        row: gpui::Stateful<gpui::Div>,
        target: DraggedRow,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let accepts = {
            let target = target.clone();
            move |dragged: &DraggedRow| dragged.group == target.group && dragged.name != target.name
        };
        row.on_drag(target.clone(), |dragged, _, _, cx| {
            cx.new(|_| dragged.clone())
        })
        .can_drop({
            let accepts = accepts.clone();
            move |value, _, _| value.downcast_ref().is_some_and(&accepts)
        })
        .drag_over::<DraggedRow>({
            let (accepts, index) = (accepts.clone(), target.index);
            let accent = theme::current(cx).colors.accent;
            move |style, dragged, _, _| {
                if !accepts(dragged) {
                    return style;
                }
                // A line where the row will land; the padding it
                // replaces keeps the row height.
                let style = style.rounded_none().border_color(accent);
                if dragged.index < index {
                    style.border_b_2().pb(px(0.))
                } else {
                    style.border_t_2().pt(px(0.))
                }
            }
        })
        .on_drop(cx.listener(move |_, dragged: &DraggedRow, window, cx| {
            if !accepts(dragged) {
                return;
            }
            let anchor = target.name.clone();
            let placement = if dragged.index < target.index {
                Placement::After(anchor)
            } else {
                Placement::Before(anchor)
            };
            window.dispatch_action(dragged.move_action(placement), cx);
        }))
    }

    /// The arrow before a label that collapses or expands its sublabels; an
    /// empty space of the same width, to keep labels aligned, for a label
    /// without any.
    fn collapse_toggle(
        &self,
        ix: usize,
        name: String,
        collapsed: Option<bool>,
        icon_color: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let toggle = div()
            .id(("sidebar-collapse", ix))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(14.));
        match collapsed {
            None => toggle,
            Some(collapsed) => {
                let pixel = theme::current(cx).is_pixel_icons();
                toggle
                    .child(
                        gpui::svg()
                            .path(assets::icon_for(
                                if collapsed {
                                    assets::CHEVRON_RIGHT
                                } else {
                                    assets::CHEVRON_DOWN
                                },
                                pixel,
                            ))
                            .size(px(14.))
                            .text_color(icon_color),
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                        // Toggling doesn't open the label.
                        cx.stop_propagation();
                        window.dispatch_action(
                            Box::new(label::ToggleCollapsed {
                                name: Some(name.clone()),
                            }),
                            cx,
                        );
                    }))
            }
        }
    }

    pub(super) fn render_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.sidebar_visible {
            return None;
        }
        let t = theme::current(cx);
        let focused = self.sidebar_focus.is_focused(window);
        // Nothing in the sidebar is being shown while the archive is.
        let active = (!self.showing_archive).then(|| self.active_sidebar_item());
        let menu_item = match &self.modal {
            Some(OpenModal {
                modal:
                    Modal::ContextMenu {
                        target: MenuTarget::Sidebar(item),
                        ..
                    },
                ..
            }) => Some(item.clone()),
            _ => None,
        };
        let rows: Vec<_> = self
            .sidebar_rows()
            .into_iter()
            .enumerate()
            .map(|(ix, row)| match row {
                Row::Heading(text, action) => div()
                    .flex()
                    .items_center()
                    .pl_3()
                    .pr_2()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.colors.text_muted)
                    .child(div().flex_1().child(text.to_uppercase()))
                    .when_some(action, |s, action| s.child(heading_button(ix, action, cx)))
                    .into_any_element(),
                Row::Item {
                    item,
                    label,
                    depth,
                    count,
                    trailing,
                    status,
                    color,
                    collapsed,
                } => {
                    let is_primary = (focused && item == self.sidebar_cursor)
                        || menu_item.as_ref() == Some(&item);
                    let is_active = active.as_ref() == Some(&item);
                    let highlight = if is_primary {
                        Some((t.colors.highlight, t.colors.highlight_text))
                    } else if is_active {
                        if t.is_bevel() {
                            Some((t.colors.highlight, t.colors.highlight_text))
                        } else {
                            Some((t.colors.hover, t.colors.text))
                        }
                    } else {
                        None
                    };
                    let row_muted = highlight.map_or(t.colors.text_muted, |(_, fg)| fg);
                    let action = item.action();
                    let muted = item == SidebarItem::ClosedProjects;
                    let reorderable = self.reorderable(&item);
                    let label_name = match &item {
                        SidebarItem::Label(name) => Some(name.clone()),
                        _ => None,
                    };
                    div()
                        .id(("sidebar", ix))
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .mx_1()
                        .pr_2()
                        .pl(px(8. + 12. * depth as f32))
                        .py(px(2.))
                        .rounded(t.radius_sm())
                        .cursor_pointer()
                        .when_some(highlight, |s, (bg, fg)| s.bg(bg).text_color(fg))
                        .when(is_primary || is_active, |s| {
                            s.children(t.glass_selection_overlay())
                        })
                        .when(focused && item == self.sidebar_cursor, |s| {
                            s.children(t.dotted_focus(t.colors.highlight_text))
                        })
                        .when(highlight.is_none(), |s| s.hover(|s| s.bg(t.colors.hover)))
                        .on_mouse_down(MouseButton::Right, {
                            let item = item.clone();
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.sidebar_cursor = item.clone();
                                this.show_context_menu(Some(event.position), window, cx);
                            })
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            // Same action as confirming the row with the keyboard;
                            // the list keeps focus so mouse users stay in the list.
                            // The action is dispatched from the sidebar: an action
                            // goes up from the element focused when it is dispatched,
                            // in the last rendered frame, where the list is missing
                            // while an issue is open.
                            this.sidebar_cursor = item.clone();
                            window.focus(&this.sidebar_focus);
                            window.dispatch_action(action.boxed_clone(), cx);
                            window.focus(&this.list_focus);
                        }))
                        .when_some(reorderable, |row, target| {
                            self.drag_and_drop(row, target, cx)
                        })
                        .when_some(status, |s, status| s.child(project_status_icon(status, &t)))
                        .when_some(label_name, |s, name| {
                            s.child(self.collapse_toggle(ix, name, collapsed, row_muted, cx))
                        })
                        .when_some(color, |s, dot| {
                            let dot_color = match dot {
                                RowDot::Accent => {
                                    if highlight.is_some() && (t.is_bevel() || t.is_crt()) {
                                        t.colors.highlight_text
                                    } else {
                                        t.colors.accent
                                    }
                                }
                                RowDot::Label(c) => t.label_color(Some(c)),
                            };
                            s.child(div().flex_none().size(px(8.)).rounded_full().bg(dot_color))
                        })
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .when(muted, |s| s.text_color(row_muted))
                                .child(truncated_text(("sidebar-label", ix), label)),
                        )
                        .when_some(
                            trailing
                                .map(str::to_string)
                                .or_else(|| (count > 0).then(|| count.to_string())),
                            |s, text| {
                                s.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(12.))
                                        .text_color(row_muted)
                                        .child(text),
                                )
                            },
                        )
                        .into_any_element()
                }
            })
            .collect();

        Some(
            div()
                .id("sidebar")
                .relative()
                .key_context(CONTEXT)
                .track_focus(&self.sidebar_focus)
                .on_action(cx.listener(Self::on_sidebar_next))
                .on_action(cx.listener(Self::on_sidebar_previous))
                .on_action(cx.listener(Self::on_sidebar_open))
                .on_action(cx.listener(Self::on_move_up))
                .on_action(cx.listener(Self::on_move_down))
                .on_action(cx.listener(Self::on_show_context_menu))
                .w(px(WIDTH))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .py_2()
                .overflow_y_scroll()
                .bg(t.colors.sidebar)
                .when(!t.is_bevel(), |s| {
                    s.border_r_1().border_color(t.colors.divider)
                })
                .children(rows)
                .children(t.sunken_bevel()),
        )
    }
}

/// The button of a section heading (e.g. + to create a project).
fn heading_button(
    ix: usize,
    action: Box<dyn Action>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    div()
        .id(("sidebar-heading-button", ix))
        .relative()
        .flex_none()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(t.radius_sm())
        .cursor_pointer()
        .when(t.is_bevel(), |s| s.bg(t.colors.surface))
        .hover(|s| s.bg(t.colors.hover))
        .child(
            gpui::svg()
                .path(assets::icon_for(assets::PLUS, t.is_pixel_icons()))
                .size(px(12.))
                .text_color(t.colors.text_muted),
        )
        .children(t.raised_bevel_thin())
        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        }))
}
