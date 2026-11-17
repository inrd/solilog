//! The history of an issue's text (`issue::ShowHistory`): a picker of its
//! revisions (earlier titles, descriptions and checklist items, newest
//! first); choosing one restores it.

use gpui::{Context, Window, prelude::*, px};
use solilog_core::{RevisionId, format_age};

use super::{Modal, Workspace};
use crate::ui::actions::issue::ShowHistory;
use crate::ui::picker::{Picker, PickerItem};

impl Workspace {
    pub(super) fn on_show_history(
        &mut self,
        _: &ShowHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issue = match self.target_issues(cx).as_slice() {
            [] => return,
            [issue] => issue.id,
            _ => {
                self.error = Some("Select a single issue to see its history".into());
                cx.notify();
                return;
            }
        };
        // Edits not saved yet are the current text: what a restore replaces.
        self.save_detail(false, cx);
        let Some(revisions) = self.report(self.db.issue_revisions(issue), cx) else {
            return;
        };
        if revisions.is_empty() {
            self.notice = Some(format!("{issue} has no earlier text"));
            cx.notify();
            return;
        }
        let items = revisions
            .iter()
            .map(|r| {
                let hint = format!(
                    "{} · {} · {}",
                    r.field_label(),
                    r.reason.label(),
                    format_age(r.created_at)
                );
                PickerItem::new(r.preview(80)).hint(Some(hint))
            })
            .collect();
        let picker = cx.new(|cx| {
            Picker::new(&format!("Restore earlier text of {issue}…"), items, 0, cx).width(px(760.))
        });
        let revisions = revisions.iter().map(|r| r.id).collect();
        self.open_modal(
            picker,
            |picker| Modal::History { picker, revisions },
            window,
            cx,
        );
    }

    /// Restores a revision; the open issue, if it is that one, shows it.
    pub(super) fn restore_revision(&mut self, id: RevisionId, cx: &mut Context<Self>) {
        self.save_detail(false, cx);
        if let Some(issue) = self.report(self.db.restore_revision(id), cx) {
            self.reload(cx);
            self.notice = Some(format!("Restored earlier text of {}", issue.id));
            cx.notify();
        }
    }
}
