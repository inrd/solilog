//! The archive: archived projects and issues, which the sidebar and the issue
//! list leave out. Opened from the command palette (`workspace::OpenArchive`);
//! each item has a context menu (right click, or `archive::ShowContextMenu`)
//! to delete it or bring it back.
//!
//! Key context: `Archive`. It takes the place of the issue list, and its
//! focus; while it is shown, "the selected issue" is the highlighted archived
//! issue.

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, MouseButton, MouseDownEvent, Pixels,
    Point, Window, div, prelude::*, px, rgb,
};
use solilog_core::{Issue, IssueChange, IssueId, Project, ProjectStatus};

use super::{MenuEntry, MenuTarget, Modal, OpenModal, Workspace, chip};
use crate::ui::actions::{archive, issue, project, workspace};
use crate::ui::picker::PickerItem;
use crate::ui::status_icon::{project_status_icon, status_icon};
use crate::ui::theme;

pub const CONTEXT: &str = "Archive";

/// A selectable archived item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveItem {
    Project(String),
    Issue(IssueId),
}

/// A rendered row: a section heading or an item.
enum Row {
    Heading(&'static str),
    Item(ArchiveItem),
}

impl Workspace {
    fn archived_projects(&self) -> impl Iterator<Item = &Project> {
        self.projects
            .iter()
            .filter(|p| p.status == ProjectStatus::Archived)
    }

    /// Archived projects (in project order), then archived issues (newest first).
    fn archive_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let projects: Vec<_> = self
            .archived_projects()
            .map(|p| Row::Item(ArchiveItem::Project(p.name.clone())))
            .collect();
        if !projects.is_empty() {
            rows.push(Row::Heading("Projects"));
            rows.extend(projects);
        }
        if !self.archived_issues.is_empty() {
            rows.push(Row::Heading("Issues"));
            rows.extend(
                self.archived_issues
                    .iter()
                    .map(|i| Row::Item(ArchiveItem::Issue(i.id))),
            );
        }
        rows
    }

    fn archive_items(&self) -> Vec<ArchiveItem> {
        self.archive_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Item(item) => Some(item),
                Row::Heading(_) => None,
            })
            .collect()
    }

    /// Index of the highlighted item among the archive's items.
    pub(super) fn archive_index(&self) -> Option<usize> {
        let cursor = self.archive_cursor.as_ref()?;
        self.archive_items().iter().position(|i| i == cursor)
    }

    /// After a reload: keeps the cursor on the same item, or on the same
    /// row (`prev_ix`) if the item left the archive.
    pub(super) fn fix_archive_cursor(&mut self, prev_ix: Option<usize>) {
        let items = self.archive_items();
        if self
            .archive_cursor
            .as_ref()
            .is_some_and(|c| items.contains(c))
        {
            return;
        }
        let ix = prev_ix.unwrap_or(0).min(items.len().saturating_sub(1));
        self.archive_cursor = items.get(ix).cloned();
    }

    /// The highlighted archived issue.
    pub(super) fn archive_selected_issue(&self) -> Option<Issue> {
        match &self.archive_cursor {
            Some(ArchiveItem::Issue(id)) => {
                self.archived_issues.iter().find(|i| i.id == *id).cloned()
            }
            _ => None,
        }
    }

    fn set_archive_cursor(&mut self, item: ArchiveItem, cx: &mut Context<Self>) {
        let row = self
            .archive_rows()
            .iter()
            .position(|r| matches!(r, Row::Item(i) if *i == item));
        if let Some(row) = row {
            self.archive_scroll.scroll_to_item(row);
        }
        self.archive_cursor = Some(item);
        cx.notify();
    }

    fn move_archive_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.archive_items();
        if items.is_empty() {
            return;
        }
        let ix = match self.archive_index() {
            Some(ix) => (ix as isize + delta).clamp(0, items.len() as isize - 1) as usize,
            None => 0,
        };
        self.set_archive_cursor(items[ix].clone(), cx);
    }

    // ---- actions ------------------------------------------------------------

    pub(super) fn on_open_archive(
        &mut self,
        _: &workspace::OpenArchive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.showing_archive = true;
        self.error = None;
        self.reload(cx);
        window.focus(&self.list_focus);
    }

    /// Back to the issue list, showing what it showed before.
    pub(super) fn close_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.showing_archive = false;
        window.focus(&self.list_focus);
        cx.notify();
    }

    fn on_close_archive(
        &mut self,
        _: &archive::Close,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_archive(window, cx);
    }

    fn on_archive_next(&mut self, _: &archive::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_archive_cursor(1, cx);
    }

    fn on_archive_previous(
        &mut self,
        _: &archive::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_archive_cursor(-1, cx);
    }

    /// Opens the menu of the highlighted item, at `position` (window
    /// coordinates) when opened with the mouse.
    fn show_archive_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.archive_cursor.clone() else {
            return;
        };
        let (title, entries): (String, Vec<MenuEntry>) = match &item {
            ArchiveItem::Project(name) => (
                name.clone(),
                vec![
                    (
                        "Set Status…",
                        Box::new(project::SetStatus {
                            name: Some(name.clone()),
                            status: None,
                        }),
                    ),
                    (
                        "Delete…",
                        Box::new(project::Delete {
                            name: Some(name.clone()),
                            confirmed: false,
                        }),
                    ),
                ],
            ),
            ArchiveItem::Issue(id) => (
                id.to_string(),
                vec![
                    ("Unarchive", Box::new(issue::Unarchive)),
                    (
                        "Delete…",
                        Box::new(issue::Delete {
                            id: Some(*id),
                            ..Default::default()
                        }),
                    ),
                ],
            ),
        };
        self.open_context_menu(
            &title,
            entries,
            MenuTarget::Archive(item),
            position,
            window,
            cx,
        );
    }

    fn on_show_archive_menu(
        &mut self,
        _: &archive::ShowContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_archive_menu(None, window, cx);
    }

    fn set_archived(&mut self, archived: bool, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.target_issues(cx).iter().map(|i| i.id).collect();
        if !ids.is_empty() {
            self.update_issues(&ids, IssueChange::SetArchived(archived), cx);
        }
    }

    pub(super) fn on_archive_issue(
        &mut self,
        _: &issue::Archive,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_archived(true, cx);
    }

    fn on_unarchive_issue(&mut self, _: &issue::Unarchive, _: &mut Window, cx: &mut Context<Self>) {
        self.set_archived(false, cx);
    }

    pub(super) fn on_delete_issue(
        &mut self,
        action: &issue::Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut ids: Vec<IssueId> = action.id.into_iter().chain(action.ids.clone()).collect();
        if ids.is_empty() {
            ids = self.target_issues(cx).iter().map(|i| i.id).collect();
            if ids.is_empty() {
                return;
            }
        }
        if !action.confirmed {
            let (prompt, item) = match ids.as_slice() {
                [id] => {
                    let Some(issue) = self.report(self.db.get_issue(*id), cx) else {
                        return;
                    };
                    (
                        format!("Delete issue {id}?"),
                        format!("Delete issue {id} “{}”", issue.title),
                    )
                }
                _ => (
                    format!("Delete {} issues?", ids.len()),
                    format!("Delete {} issues", ids.len()),
                ),
            };
            let item = PickerItem::new(item).hint(Some("can't be undone"));
            let delete = issue::Delete {
                id: None,
                ids,
                confirmed: true,
            };
            return self.open_action_picker(&prompt, vec![(item, Box::new(delete))], 0, window, cx);
        }
        if self.report(self.db.delete_issues(&ids), cx).is_some() {
            self.reload(cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    fn render_archive_item(
        &self,
        ix: usize,
        item: ArchiveItem,
        highlight: Option<u32>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = div()
            .id(("archive", ix))
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_1()
            .when_some(highlight, |s, color| s.bg(rgb(color)))
            .on_click(cx.listener({
                let item = item.clone();
                move |this, _: &ClickEvent, window, cx| {
                    this.set_archive_cursor(item.clone(), cx);
                    window.focus(&this.list_focus);
                }
            }))
            .on_mouse_down(MouseButton::Right, {
                let item = item.clone();
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.set_archive_cursor(item.clone(), cx);
                    this.show_archive_menu(Some(event.position), window, cx);
                })
            });
        match item {
            ArchiveItem::Project(name) => {
                let count = self
                    .projects
                    .iter()
                    .find(|p| p.name == name)
                    .map_or(0, |p| p.issue_count);
                row.child(
                    div()
                        .w(px(20.))
                        .flex()
                        .flex_none()
                        .justify_center()
                        .child(project_status_icon(ProjectStatus::Archived)),
                )
                .child(div().flex_1().truncate().child(name))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(match count {
                            1 => "1 issue".to_string(),
                            n => format!("{n} issues"),
                        }),
                )
                .into_any_element()
            }
            ArchiveItem::Issue(id) => {
                let Some(issue) = self.archived_issues.iter().find(|i| i.id == id) else {
                    return row.into_any_element();
                };
                row.child(
                    div()
                        .w(px(20.))
                        .flex()
                        .flex_none()
                        .justify_center()
                        .child(status_icon(issue.status)),
                )
                .child(
                    div()
                        .w(px(52.))
                        .flex_none()
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(id.to_string()),
                )
                .child(div().flex_1().truncate().child(issue.title.clone()))
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
                .into_any_element()
            }
        }
    }

    /// Takes the place of the new-issue input and the issue list.
    pub(super) fn render_archive(
        &self,
        list_focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let menu_item = match &self.modal {
            Some(OpenModal {
                modal:
                    Modal::ContextMenu {
                        target: MenuTarget::Archive(item),
                        ..
                    },
                ..
            }) => Some(item.clone()),
            _ => None,
        };
        let rows = self.archive_rows();
        let empty = rows.is_empty();
        let count = self.archive_items().len();
        let rows: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, row)| match row {
                Row::Heading(text) => div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(text.to_uppercase())
                    .into_any_element(),
                Row::Item(item) => {
                    let highlight = if menu_item.as_ref() == Some(&item)
                        || (list_focused && self.archive_cursor.as_ref() == Some(&item))
                    {
                        Some(theme::HIGHLIGHT)
                    } else if self.archive_cursor.as_ref() == Some(&item) {
                        Some(theme::HOVER)
                    } else {
                        None
                    };
                    self.render_archive_item(ix, item, highlight, cx)
                }
            })
            .collect();

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(theme::DIVIDER))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Archive"))
                    .child(
                        div()
                            .ml_3()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("Right-click an item to delete it or bring it back."),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .pl_3()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(count.to_string()),
                    ),
            )
            .child(
                div()
                    .id("archive")
                    .key_context(CONTEXT)
                    .track_focus(&self.list_focus)
                    .on_action(cx.listener(Self::on_archive_next))
                    .on_action(cx.listener(Self::on_archive_previous))
                    .on_action(cx.listener(Self::on_show_archive_menu))
                    .on_action(cx.listener(Self::on_close_archive))
                    .on_action(cx.listener(Self::on_unarchive_issue))
                    .on_action(cx.listener(Self::on_delete_issue))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.archive_scroll)
                    .pb_1()
                    .children(rows)
                    .when(empty, |s| {
                        s.child(
                            div()
                                .px_4()
                                .py_2()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child("Nothing archived."),
                        )
                    }),
            )
    }
}
