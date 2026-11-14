//! The sidebar: everything the issue list can show (all issues, saved views,
//! projects, labels), with the number of open issues in each. Done and
//! archived projects sit in a collapsed section.
//!
//! Key context: `Sidebar`. Each row stands for an action (e.g. `view::Open`);
//! confirming a row, by key or click, dispatches it.

use gpui::{
    Action, ClickEvent, Context, FontWeight, IntoElement, SharedString, Window, div, prelude::*,
    px, rgb,
};
use solilog_core::IssueFilter;

use super::Workspace;
use super::scope::Source;
use crate::ui::actions::{sidebar, view, workspace};
use crate::ui::theme;

pub const CONTEXT: &str = "Sidebar";
const WIDTH: f32 = 220.;

/// A selectable sidebar row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItem {
    AllIssues,
    View(String),
    Project(String),
    /// Expands or collapses the done and archived projects.
    ClosedProjects,
    Label(String),
}

impl SidebarItem {
    /// What confirming the row does.
    fn action(&self) -> Box<dyn Action> {
        match self {
            SidebarItem::AllIssues => Box::new(view::ShowAll),
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

/// A rendered row: a section heading or an item.
enum Row {
    Heading(&'static str),
    Item {
        item: SidebarItem,
        label: SharedString,
        /// Nesting level, for the label tree.
        depth: usize,
        count: usize,
        /// Overrides `count` on the right (e.g. the closed-projects chevron).
        trailing: Option<&'static str>,
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
        }];
        if !self.views.is_empty() {
            rows.push(Row::Heading("Views"));
            rows.extend(self.views.iter().map(|v| Row::Item {
                item: SidebarItem::View(v.name.clone()),
                label: v.name.clone().into(),
                depth: 0,
                count: self.open_count(&v.filter),
                trailing: None,
            }));
        }

        if !self.projects.is_empty() {
            rows.push(Row::Heading("Projects"));
            let project_row = |p: &solilog_core::Project| Row::Item {
                item: SidebarItem::Project(p.name.clone()),
                label: p.name.clone().into(),
                depth: 0,
                count: self.open_count(&Source::Project(p.name.clone()).filter()),
                trailing: None,
            };
            let (closed, open): (Vec<_>, Vec<_>) =
                self.projects.iter().partition(|p| p.status.is_closed());
            rows.extend(open.into_iter().map(project_row));
            if !closed.is_empty() {
                rows.push(Row::Item {
                    item: SidebarItem::ClosedProjects,
                    label: format!("Closed ({})", closed.len()).into(),
                    depth: 0,
                    count: 0,
                    trailing: Some(if self.show_closed_projects {
                        "▾"
                    } else {
                        "▸"
                    }),
                });
                if self.show_closed_projects {
                    rows.extend(closed.into_iter().map(|p| match project_row(p) {
                        Row::Item {
                            item, label, count, ..
                        } => Row::Item {
                            item,
                            label,
                            depth: 1,
                            count,
                            trailing: None,
                        },
                        heading => heading,
                    }));
                }
            }
        }

        if !self.labels.is_empty() {
            rows.push(Row::Heading("Labels"));
            rows.extend(self.labels.iter().map(|l| {
                let leaf = l.name.rsplit(solilog_core::LABEL_SEPARATOR).next();
                Row::Item {
                    item: SidebarItem::Label(l.name.clone()),
                    label: leaf.unwrap_or(&l.name).to_string().into(),
                    depth: l.depth(),
                    count: self.open_count(&Source::Label(l.name.clone()).filter()),
                    trailing: None,
                }
            }));
        }
        rows
    }

    /// The selectable items, in display order.
    pub(super) fn sidebar_items(&self) -> Vec<SidebarItem> {
        self.sidebar_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Item { item, .. } => Some(item),
                Row::Heading(_) => None,
            })
            .collect()
    }

    /// The item matching what the issue list currently shows.
    pub(super) fn active_sidebar_item(&self) -> SidebarItem {
        match &self.scope.source {
            Source::All => SidebarItem::AllIssues,
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

    fn on_sidebar_open(&mut self, _: &sidebar::Open, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(self.sidebar_cursor.action(), cx);
    }

    pub(super) fn render_sidebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if !self.sidebar_visible {
            return None;
        }
        let focused = self.sidebar_focus.is_focused(window);
        let active = self.active_sidebar_item();
        let rows: Vec<_> = self
            .sidebar_rows()
            .into_iter()
            .enumerate()
            .map(|(ix, row)| match row {
                Row::Heading(text) => div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(text.to_uppercase())
                    .into_any_element(),
                Row::Item {
                    item,
                    label,
                    depth,
                    count,
                    trailing,
                } => {
                    let highlight = if focused && item == self.sidebar_cursor {
                        Some(theme::HIGHLIGHT)
                    } else if item == active {
                        Some(theme::HOVER)
                    } else {
                        None
                    };
                    let action = item.action();
                    let muted = item == SidebarItem::ClosedProjects;
                    div()
                        .id(("sidebar", ix))
                        .flex()
                        .items_center()
                        .gap_2()
                        .mx_1()
                        .pr_2()
                        .pl(px(8. + 12. * depth as f32))
                        .py(px(2.))
                        .rounded_sm()
                        .cursor_pointer()
                        .when_some(highlight, |s, color| s.bg(rgb(color)))
                        .hover(|s| s.bg(rgb(theme::HOVER)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            // Same action as confirming the row with the keyboard;
                            // the list keeps focus so mouse users stay in the list.
                            this.sidebar_cursor = item.clone();
                            window.focus(&this.list_focus);
                            window.dispatch_action(action.boxed_clone(), cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .when(muted, |s| s.text_color(rgb(theme::TEXT_MUTED)))
                                .child(label),
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
                                        .text_color(rgb(theme::TEXT_MUTED))
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
                .key_context(CONTEXT)
                .track_focus(&self.sidebar_focus)
                .on_action(cx.listener(Self::on_sidebar_next))
                .on_action(cx.listener(Self::on_sidebar_previous))
                .on_action(cx.listener(Self::on_sidebar_open))
                .w(px(WIDTH))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .py_2()
                .overflow_y_scroll()
                .bg(rgb(theme::SIDEBAR))
                .border_r_1()
                .border_color(rgb(theme::DIVIDER))
                .children(rows),
        )
    }
}
