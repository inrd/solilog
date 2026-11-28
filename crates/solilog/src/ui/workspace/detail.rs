//! The issue detail: one issue, in place of the issue list (or the archive),
//! with its title, description and checklist editable in place and its
//! notes in the main column; its facts, projects, labels, links,
//! attachments and dependencies in a side pane (`aside.rs`). Opened with
//! `issue::Open` (Enter or double click in the list), left with
//! `issue_detail::Close` (Escape).
//!
//! Key context: `IssueDetail`. Its pane has the focus while no field is
//! being edited; issue commands act on the open issue. Edits are saved
//! shortly after typing stops, when a field loses focus, and when the detail
//! closes. Changes made elsewhere (e.g. from the CLI) show up in a field
//! unless it has unsaved edits.
//!
//! Issue ids in the description (`SOL-12`) link to their issues, shown as
//! their titles except while the cursor is on them; web addresses open in
//! the browser.

mod aside;
mod attachments;
mod checklist;
mod dependencies;
mod links;
mod notes;
mod overwrite;
mod preview;
mod review;

use std::ops::Range;
use std::time::Duration;

use gpui::{
    Action, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight,
    IntoElement, ScrollHandle, Subscription, Task, Window, div, prelude::*, px,
};
use solilog_core::{
    Activity, Db, Decision, DecisionOwner, Issue, IssueEdit, IssueId, Note, format_age,
    issue_mentions, normalize_description, web_links,
};

use super::Workspace;
use super::section::issue_section;
use crate::ui::actions::{
    attachment,
    issue::{self, SetStatus},
    issue_detail, workspace,
};
use crate::ui::status_icon::status_icon;
use crate::ui::text_input::{Changed, Link, Submit, TextInput};
use crate::ui::theme;
use checklist::Checklist;
use notes::Notes;
use overwrite::{Overwrites, TextField};
use preview::ImagePreview;

pub const CONTEXT: &str = "IssueDetail";

/// How long after the last keystroke edits are saved.
pub(super) const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);

pub(super) struct IssueDetail {
    /// The issue as last read from (or written to) the database.
    issue: Issue,
    /// Non-user activities since this issue was last marked reviewed, newest first.
    unreviewed_activities: Vec<Activity>,
    title: Entity<TextInput>,
    description: Entity<TextInput>,
    checklist: Checklist,
    notes: Notes,
    /// The decisions that apply, then ended ones awaiting review (see
    /// `Db::shown_decisions`).
    decisions: Vec<Decision>,
    /// The other issues that mention this one (`Db::mentioned_by`).
    mentioned_by: Vec<IssueId>,
    /// Edits made elsewhere while the user typed (see `overwrite.rs`).
    overwrites: Overwrites,
    /// The image attachment shown over the window, if any.
    preview: Option<ImagePreview>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    autosave: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl IssueDetail {
    pub(super) fn issue(&self) -> &Issue {
        &self.issue
    }

    pub(super) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(super) fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    pub(super) fn mentioned_by(&self) -> &[IssueId] {
        &self.mentioned_by
    }

    /// The user opened the history: edits made elsewhere that their text
    /// replaced are no longer shown.
    pub(super) fn dismiss_overwrites(&mut self) {
        self.overwrites.dismiss();
    }
}

/// Whether the title field holds something other than `saved`.
fn title_changed(text: &str, saved: &str) -> bool {
    text.trim() != saved
}

pub(super) fn description_changed(text: &str, saved: &str) -> bool {
    normalize_description(text) != saved
}

/// What the detail shows besides the issue, read from the database.
struct Extras {
    unreviewed_activities: Vec<Activity>,
    notes: Vec<Note>,
    decisions: Vec<Decision>,
    mentioned_by: Vec<IssueId>,
}

impl Extras {
    fn read(db: &Db, id: IssueId) -> anyhow::Result<Self> {
        Ok(Self {
            unreviewed_activities: db.unreviewed_activities(id)?,
            notes: db.issue_notes(id)?,
            decisions: db.shown_decisions(&DecisionOwner::issue(id))?,
            mentioned_by: db.mentioned_by(id)?,
        })
    }
}

impl Workspace {
    pub(super) fn open_issue(&mut self, id: IssueId, window: &mut Window, cx: &mut Context<Self>) {
        let read = self
            .db
            .get_issue(id)
            .and_then(|issue| Ok((issue, Extras::read(&self.db, id)?)));
        let Some((issue, extras)) = self.report(read, cx) else {
            return;
        };
        self.close_detail(cx);
        let scroll = ScrollHandle::new();
        let title = cx.new(|cx| {
            let mut input = TextInput::new("Title", cx).wrap();
            input.reset_text(issue.title.clone(), cx);
            input
        });
        let workspace = cx.weak_entity();
        let description = cx.new(|cx| {
            let mut input = TextInput::new("Add a description…", cx)
                .multiline()
                .autoscroll(scroll.clone())
                .links(move |text, cx| {
                    workspace
                        .upgrade()
                        .map(|w| w.read(cx).description_links(text))
                        .unwrap_or_default()
                });
            input.reset_text(issue.description.clone(), cx);
            input
        });
        // Blur listeners run while the window draws, when changes don't
        // trigger a redraw: save right after instead.
        let subscriptions = vec![
            cx.subscribe_in(&title, window, |this, _, _: &Changed, window, cx| {
                this.schedule_autosave(window, cx)
            }),
            cx.subscribe_in(&description, window, |this, _, _: &Changed, window, cx| {
                this.schedule_autosave(window, cx)
            }),
            // Enter in the title moves on to the description.
            cx.subscribe_in(&title, window, |this, _, _: &Submit, window, cx| {
                this.edit_detail_field(false, window, cx)
            }),
            cx.on_blur(&title.focus_handle(cx), window, |_, window, cx| {
                cx.defer_in(window, |this, _, cx| this.save_detail(true, cx))
            }),
            cx.on_blur(&description.focus_handle(cx), window, |_, window, cx| {
                cx.defer_in(window, |this, _, cx| this.save_detail(true, cx))
            }),
        ];
        let checklist = Checklist::new(window, cx);
        let notes = Notes::new(extras.notes, scroll.clone(), cx);
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.detail = Some(IssueDetail {
            issue,
            unreviewed_activities: extras.unreviewed_activities,
            title,
            description,
            checklist,
            notes,
            decisions: extras.decisions,
            mentioned_by: extras.mentioned_by,
            overwrites: Overwrites::default(),
            preview: None,
            focus,
            scroll,
            autosave: None,
            _subscriptions: subscriptions,
        });
        self.footer.error = None;
        self.sync_checklist(window, cx);
        cx.notify();
    }

    /// The links in a description: web addresses, opened in the browser,
    /// and the issues mentioned (archived ones too), shown as their titles.
    /// Mentions of issues that don't exist stay plain text, and so do those
    /// within a web address.
    pub(super) fn description_links(&self, text: &str) -> Vec<Link> {
        let web = web_links(text);
        let in_web_link = |range: &Range<usize>| {
            web.iter()
                .any(|w| w.start < range.end && range.start < w.end)
        };
        let issues = issue_mentions(text)
            .into_iter()
            .filter(|(range, _)| !in_web_link(range))
            .filter_map(|(range, id)| {
                let issue = self
                    .all_issues
                    .iter()
                    .chain(&self.archive.issues)
                    .find(|i| i.id == id)?;
                Some(Link {
                    range,
                    label: Some(issue.title.clone().into()),
                    strikethrough: issue.status.is_closed(),
                    action: Box::new(issue::Open { id: Some(id) }),
                })
            });
        let mut links: Vec<Link> = web
            .iter()
            .map(|range| Link {
                range: range.clone(),
                label: None,
                strikethrough: false,
                action: Box::new(workspace::OpenUrl {
                    url: text[range.clone()].to_string(),
                }),
            })
            .chain(issues)
            .collect();
        links.sort_by_key(|link| link.range.start);
        links
    }

    fn schedule_autosave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        detail.autosave = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            this.update(cx, |this, cx| this.save_detail(false, cx)).ok();
        }));
    }

    /// Writes the edited title, description and checklist items, if
    /// changed. `leaving`: the user is done with the field (it lost focus,
    /// the detail closes); an empty title then goes back to the saved one,
    /// with an error, and an emptied item is deleted (see `checklist.rs`).
    /// While typing, they're just not saved yet.
    pub(super) fn save_detail(&mut self, leaving: bool, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let id = detail.issue.id;
        let title = detail.title.read(cx).text().to_owned();
        let description = detail.description.read(cx).text().to_owned();
        let mut edit = IssueEdit {
            title: title_changed(&title, &detail.issue.title).then_some(title),
            description: description_changed(&description, &detail.issue.description)
                .then_some(description),
            ..Default::default()
        };
        if edit.title.as_ref().is_some_and(|t| t.trim().is_empty()) {
            edit.title = None;
            if leaving {
                let saved = detail.issue.title.clone();
                detail
                    .title
                    .update(cx, |input, cx| input.reset_text(saved, cx));
                self.footer.error = Some("An issue needs a title".into());
                cx.notify();
            }
        }
        let mut wrote = false;
        if edit != IssueEdit::default() {
            let Some(issue) = self.report(self.db.edit_issue(id, &edit), cx) else {
                return;
            };
            if let Some(detail) = &mut self.detail {
                detail.issue = issue;
                if edit.title.is_some() {
                    detail.overwrites.saved(TextField::Title);
                }
                if edit.description.is_some() {
                    detail.overwrites.saved(TextField::Description);
                }
            }
            wrote = true;
        }
        if self.save_checklist(leaving, cx) || wrote {
            self.reload(cx);
        }
    }

    /// After a reload: shows the issue as it now is. Fields with unsaved
    /// edits keep them, and so do fields that differ from the saved text
    /// only by the whitespace saving drops (a space just typed at the end
    /// must stay); one changed elsewhere meanwhile is noted (see
    /// `overwrite.rs`). Closes the detail if the issue is gone; on other
    /// errors, reports them and leaves the detail as it was.
    pub(super) fn refresh_detail(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.detail.as_ref().map(|d| d.issue.id) else {
            return;
        };
        let read = self.db.find_issue(id).and_then(|issue| match issue {
            Some(issue) => Ok(Some((issue, Extras::read(&self.db, id)?))),
            None => Ok(None),
        });
        let (issue, extras) = match read {
            Ok(Some(read)) => read,
            Ok(None) => {
                // Deleted. Focus goes back to the list (see `Workspace::new`).
                self.detail = None;
                return;
            }
            Err(err) => {
                // Not `report`: on success it would clear an error the
                // reload found before this.
                self.footer.error = Some(format!("{err:#}"));
                return;
            }
        };
        let Some(detail) = &mut self.detail else {
            return;
        };
        let title = detail.title.read(cx).text();
        let unsaved = title_changed(title, &detail.issue.title);
        detail
            .overwrites
            .refresh(TextField::Title, unsaved, &detail.issue, &issue);
        if !unsaved && title_changed(title, &issue.title) {
            let text = issue.title.clone();
            detail
                .title
                .update(cx, |input, cx| input.reset_text(text, cx));
        }
        let description = detail.description.read(cx).text();
        let unsaved = description_changed(description, &detail.issue.description);
        detail
            .overwrites
            .refresh(TextField::Description, unsaved, &detail.issue, &issue);
        if !unsaved && description_changed(description, &issue.description) {
            let text = issue.description.clone();
            detail
                .description
                .update(cx, |input, cx| input.reset_text(text, cx));
        }
        detail.issue = issue;
        detail.unreviewed_activities = extras.unreviewed_activities;
        detail.notes.set_notes(extras.notes);
        detail.decisions = extras.decisions;
        detail.mentioned_by = extras.mentioned_by;
    }

    /// Saves (adding the note being written) and closes the detail; the
    /// caller moves the focus.
    pub(super) fn close_detail(&mut self, cx: &mut Context<Self>) {
        if self.detail.is_some() {
            self.save_detail(true, cx);
            self.add_drafted_note(cx);
            self.detail = None;
            cx.notify();
        }
    }

    /// Opens the issue, unless it is open already.
    pub(super) fn show_issue(&mut self, id: IssueId, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail.as_ref().is_none_or(|d| d.issue.id != id) {
            self.open_issue(id, window, cx);
        }
    }

    /// Puts the cursor at the end of the title or the description.
    fn edit_detail_field(&mut self, title: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let input = if title {
            &detail.title
        } else {
            &detail.description
        };
        window.focus(&input.focus_handle(cx));
        input.update(cx, |input, cx| input.move_to_end(cx));
    }

    // ---- actions ------------------------------------------------------------

    pub(super) fn on_open_issue(
        &mut self,
        action: &issue::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = match action.id {
            Some(id) => id,
            None => match self.selected_issue(cx) {
                Some(issue) => issue.id,
                None => return,
            },
        };
        self.open_issue(id, window, cx);
    }

    fn on_close_detail(
        &mut self,
        _: &issue_detail::Close,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_detail(cx);
        window.focus(&self.list_focus);
    }

    fn on_edit_title(
        &mut self,
        _: &issue_detail::EditTitle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_detail_field(true, window, cx);
    }

    fn on_edit_description(
        &mut self,
        _: &issue_detail::EditDescription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_detail_field(false, window, cx);
    }

    fn on_stop_editing(
        &mut self,
        _: &issue_detail::StopEditing,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        // Saving happens when the field loses focus.
        if let Some(detail) = &self.detail {
            window.focus(&detail.focus);
        }
    }

    // ---- rendering ------------------------------------------------------------

    /// Takes the place of the new-issue input and the issue list.
    pub(super) fn render_detail(
        &self,
        detail: &IssueDetail,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme::current(cx);
        let sidebar = if self.sidebar.visible {
            super::sidebar::WIDTH
        } else {
            0.
        };
        let available = f32::from(window.viewport_size().width) - sidebar;
        let two_columns = available >= aside::MIN_MAIN_WIDTH + aside::WIDTH;
        let issue = &detail.issue;
        let focus = detail.focus.clone();
        let back = div()
            .id("detail-back")
            .relative()
            .px_2()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .when(t.is_bevel(), |s| s.bg(t.colors.surface))
            .text_color(if t.is_bevel() {
                t.colors.text
            } else {
                t.colors.text_muted
            })
            .hover(|s| s.bg(t.colors.hover).text_color(t.colors.text))
            .when(t.is_bevel(), |s| s.active(|s| s.pt(px(1.)).pl(px(9.))))
            .child("‹ Issues")
            .children(t.raised_bevel_thin())
            .on_click({
                let focus = focus.clone();
                move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(issue_detail::Close.boxed_clone(), cx);
                }
            });
        // Margins, not a gap: the row's gap doesn't show alongside ml_auto.
        let header = div()
            .flex()
            .items_center()
            .px_2()
            .py_3()
            .when(t.surface_toolbars(), |s| s.bg(t.colors.surface))
            .border_b_1()
            .border_color(t.colors.divider)
            .child(back)
            .child(
                div()
                    .ml_3()
                    .text_color(t.colors.text_muted)
                    .child(issue.id.to_string()),
            )
            .when(issue.title_by.is_agent(), |s| {
                s.child(
                    div()
                        .ml_3()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(format!("by {}", issue.title_by)),
                )
            })
            .child(
                div()
                    .ml_auto()
                    .pl_3()
                    .pr_2()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(t.colors.text_muted)
                    .child("Saved as you type"),
            );

        let status = div()
            .id("detail-status")
            .relative()
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .when(t.is_bevel(), |s| s.bg(t.colors.surface))
            .hover(|s| s.bg(t.colors.hover))
            .child(status_icon(issue.status, &t))
            .children(t.raised_bevel_thin())
            .on_click({
                let focus = focus.clone();
                move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(SetStatus::default().boxed_clone(), cx);
                }
            });
        let unreviewed_banner = issue.unreviewed.then(|| {
            const MAX_SHOWN: usize = 5;
            let focus = focus.clone();
            let shown = detail.unreviewed_activities.iter().take(MAX_SHOWN);
            let extra = detail.unreviewed_activities.len().saturating_sub(MAX_SHOWN);
            div()
                .relative()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .rounded(t.radius_sm())
                .bg(t.colors.surface)
                .border_l_2()
                .border_color(t.colors.accent)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.colors.accent)
                                .child(if issue.awaiting_triage {
                                    "Awaiting triage"
                                } else {
                                    "Unreviewed changes"
                                }),
                        )
                        // An agent filed it: accepting is marking it
                        // reviewed; declining cancels it.
                        .when(issue.awaiting_triage, |s| {
                            s.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(t.colors.text_muted)
                                    .child(format!("filed by {}", issue.created_by)),
                            )
                        })
                        .child(div().flex_1())
                        .when(issue.awaiting_triage, |s| {
                            s.child(review::button(
                                "detail-decline",
                                "Decline",
                                false,
                                &focus,
                                &t,
                                Box::new(issue::Decline),
                            ))
                        })
                        .child(review::button(
                            "detail-mark-reviewed",
                            if issue.awaiting_triage {
                                "Accept"
                            } else {
                                "Mark reviewed"
                            },
                            false,
                            &focus,
                            &t,
                            Box::new(issue::MarkReviewed),
                        )),
                )
                .children(shown.map(|a| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(
                            div()
                                .flex_none()
                                .text_color(t.colors.text)
                                .child(a.actor.to_string()),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(a.summary()))
                        .child(div().flex_none().child(format_age(a.created_at)))
                }))
                .when(extra > 0, |s| {
                    s.child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.colors.text_muted)
                            .child(format!("+{extra} more")),
                    )
                })
                .children(t.raised_bevel_thin())
        });

        let review_panel = self.render_review_panel(detail, &focus, &t);
        let overwrites = overwrite::render_overwrites(detail, &focus, &t);

        let agent_description = !issue.description.is_empty() && issue.description_by.is_agent();

        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_w_0()
            .px_6()
            .py_5()
            .child(
                // The status stays by the title's first line.
                div().flex().items_start().gap_2().child(status).child(
                    // A column, so the wrapped title is measured at its width.
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(20.))
                        .line_height(px(28.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(detail.title.clone()),
                ),
            )
            .children(review_panel)
            .children(overwrites)
            .children(unreviewed_banner)
            .child(
                div()
                    .relative()
                    .mt_2()
                    .flex()
                    .flex_col()
                    // Explicit `min_h` replaces flexbox's content-based min
                    // height, so without `flex_shrink_0` this section shrinks
                    // below its wrapped text in wide windows and leaks into
                    // what follows.
                    .flex_shrink_0()
                    .min_h(px(120.))
                    .line_height(px(22.))
                    .when(t.is_bevel(), |s| s.px_2().py_1().bg(t.colors.input))
                    .when(agent_description, |s| {
                        s.pl_3()
                            .border_l_2()
                            .border_color(if issue.unreviewed {
                                t.colors.accent
                            } else {
                                t.colors.border
                            })
                            .child(
                                div()
                                    .mb_1()
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(t.colors.text_muted)
                                    .child(format!("by {}", issue.description_by)),
                            )
                    })
                    .child(detail.description.clone())
                    .children(t.sunken_bevel()),
            )
            .when(!two_columns, |s| {
                let sections = self.render_aside_sections(detail, cx);
                s.children(sections.into_iter().map(|s| issue_section(s, &t)))
            })
            .children(
                self.render_decisions(&detail.decisions, &DecisionOwner::issue(issue.id), true, cx)
                    .map(|d| issue_section(d, &t)),
            )
            .child(issue_section(self.render_checklist(detail, cx), &t))
            .child(issue_section(self.render_notes(detail, cx), &t));

        let aside = two_columns.then(|| {
            div()
                .id("issue-detail-aside")
                .flex_none()
                .w(px(aside::WIDTH))
                .min_h_0()
                .overflow_y_scroll()
                .border_l_1()
                .border_color(t.colors.divider)
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .px_4()
                        .py_5()
                        .children(
                            self.render_aside_sections(detail, cx)
                                .into_iter()
                                .enumerate()
                                .map(|(i, s)| {
                                    if i == 0 {
                                        s
                                    } else {
                                        issue_section(s, &t).into_any_element()
                                    }
                                }),
                        ),
                )
        });

        let drag_bg = t.colors.selection;
        div()
            .flex_1()
            .min_w_0()
            // Without it, the column grows to fit the whole issue and the
            // scroll area below never overflows: the wheel does nothing.
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("issue-detail")
                    .key_context(CONTEXT)
                    .track_focus(&focus)
                    .on_action(cx.listener(Self::on_close_detail))
                    .on_action(cx.listener(Self::on_edit_title))
                    .on_action(cx.listener(Self::on_edit_description))
                    .on_action(cx.listener(Self::on_stop_editing))
                    .on_action(cx.listener(Self::on_archive_issue))
                    .on_action(cx.listener(Self::on_unarchive_issue))
                    .on_action(cx.listener(Self::on_delete_issue))
                    .on_action(cx.listener(Self::on_show_issue_menu))
                    .on_action(cx.listener(Self::on_toggle_checklist_item))
                    .on_action(cx.listener(Self::on_delete_checklist_item))
                    .on_action(cx.listener(Self::on_move_item))
                    .on_action(cx.listener(Self::on_add_attachments))
                    .on_action(cx.listener(Self::on_paste_attachments))
                    .on_action(cx.listener(Self::on_reveal_attachment))
                    .on_action(cx.listener(Self::on_delete_attachment))
                    .on_action(cx.listener(Self::on_preview_attachment))
                    .on_action(cx.listener(Self::on_delete_note))
                    // Files dropped anywhere on the issue are attached to it.
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drag_bg))
                    .on_drop({
                        let focus = focus.clone();
                        move |paths: &ExternalPaths, window, cx| {
                            let action = attachment::Add {
                                paths: paths.paths().to_vec(),
                            };
                            focus.dispatch_action(&action, window, cx);
                        }
                    })
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_detail_main(body, detail))
                    .children(aside),
            )
    }

    /// The main column's scroll area.
    fn render_detail_main(&self, body: gpui::Div, detail: &IssueDetail) -> impl IntoElement {
        div()
            .id("issue-detail-main")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&detail.scroll)
            // The scroll range is the height of the row below, so
            // it must be measured at the width the text is drawn:
            // - a flex column stretches the row to the area's width
            //   (as a plain block, it was sized from an unwrapped
            //   pass: too short to reach the bottom);
            // - the row, not the body, has the max width, so the
            //   row's width is known (the area's, capped) whenever
            //   its height is measured (with the cap on the body,
            //   the height came from passes at other widths:
            //   unwrapped, or zero with one character per line,
            //   thousands of pixels too long);
            // - the body is a flex item of the row, so it fills the
            //   row (stretched or `w_full`, it was measured wider
            //   than drawn, too short, and ran into what follows it);
            // - `flex_shrink_0` keeps the row from shrinking to the
            //   area's height.
            .flex()
            .flex_col()
            .child(div().flex().flex_shrink_0().max_w(px(760.)).child(body))
    }
}
