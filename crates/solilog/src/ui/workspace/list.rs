//! The issue list's state: the listed issues in their status sections, the
//! highlighted issue and the issues selected for commands. Plain data, so
//! its logic is testable without a window.

use std::collections::HashSet;

use gpui::ScrollHandle;
use solilog_core::{Issue, IssueId, Status};

#[derive(Default)]
pub(super) struct ListState {
    /// The listed issues, as shown: grouped by status, without collapsed
    /// sections.
    pub issues: Vec<Issue>,
    /// The list's status sections, in list order, with their number of
    /// issues; `issues` leaves out those of collapsed sections.
    pub sections: Vec<(Status, usize)>,
    /// Statuses whose section is collapsed.
    pub collapsed_statuses: Vec<Status>,
    /// The highlighted issue (the keyboard cursor).
    pub selected: Option<IssueId>,
    /// Issues selected for commands, which act on these when there are any
    /// and on the highlighted issue otherwise. Only ever listed issues:
    /// `set_issues` drops the others.
    pub selection: HashSet<IssueId>,
    /// Where the range being selected (shift + move or click) started.
    pub selection_anchor: Option<IssueId>,
    pub scroll: ScrollHandle,
}

impl ListState {
    /// Lists `issues`, grouped by status (keeping their order within a
    /// status), leaving out collapsed sections. The highlight stays on the
    /// same issue, or on the row at `prev_ix` if it is no longer listed.
    pub fn set_issues(&mut self, mut issues: Vec<Issue>, prev_ix: Option<usize>) {
        issues.sort_by_key(|i| i.status.list_rank());
        self.sections = issues
            .chunk_by(|a, b| a.status == b.status)
            .map(|group| (group[0].status, group.len()))
            .collect();
        issues.retain(|i| !self.collapsed_statuses.contains(&i.status));
        self.issues = issues;
        let listed: HashSet<IssueId> = self.issues.iter().map(|i| i.id).collect();
        self.selection.retain(|id| listed.contains(id));
        self.selection_anchor = self.selection_anchor.filter(|id| listed.contains(id));
        if self.selected_index().is_none() {
            let ix = prev_ix.unwrap_or(0).min(self.last_index());
            self.selected = self.issues.get(ix).map(|i| i.id);
        }
    }

    /// Lists nothing (the issues couldn't be read).
    pub fn clear(&mut self) {
        self.issues.clear();
        self.sections.clear();
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.issues.iter().position(|i| i.id == id)
    }

    /// The highlighted issue, if it is listed.
    pub fn highlighted(&self) -> Option<&Issue> {
        self.selected_index().map(|ix| &self.issues[ix])
    }

    /// The index of the last listed issue (0 when there are none).
    pub fn last_index(&self) -> usize {
        self.issues.len().saturating_sub(1)
    }

    /// The list's child element showing the issue at `ix`: rows are
    /// preceded by one header per status section, up to and including
    /// theirs (collapsed sections have a header too).
    pub fn child_index(&self, ix: usize) -> usize {
        let Some(issue) = self.issues.get(ix) else {
            return ix;
        };
        let headers = self
            .sections
            .iter()
            .take_while(|(s, _)| s.list_rank() <= issue.status.list_rank())
            .count();
        ix + headers
    }

    /// Scrolls the highlighted issue into view.
    pub fn scroll_to_selected(&self) {
        if let Some(ix) = self.selected_index() {
            self.scroll.scroll_to_item(self.child_index(ix));
        }
    }

    /// Highlights the issue at `ix`, scrolling it into view. False if
    /// there is none there.
    pub fn highlight(&mut self, ix: usize) -> bool {
        let Some(issue) = self.issues.get(ix) else {
            return false;
        };
        self.selected = Some(issue.id);
        self.scroll.scroll_to_item(self.child_index(ix));
        true
    }

    /// Highlights the issue at `ix`; a later range selection starts there.
    pub fn select(&mut self, ix: usize) -> bool {
        self.selection_anchor = None;
        self.highlight(ix)
    }

    /// Moves the highlight to `ix`, selecting the issues from the anchor
    /// (where the range started) to it, instead of the previous range.
    pub fn extend_selection_to(&mut self, ix: usize) -> bool {
        let Some(cursor) = self.selected_index() else {
            return self.select(ix);
        };
        let anchor = self
            .selection_anchor
            .and_then(|id| self.issues.iter().position(|i| i.id == id))
            .unwrap_or(cursor);
        let ix = ix.min(self.last_index());
        let range = |end: usize| anchor.min(end)..=anchor.max(end);
        for issue in &self.issues[range(cursor)] {
            self.selection.remove(&issue.id);
        }
        for issue in &self.issues[range(ix)] {
            self.selection.insert(issue.id);
        }
        self.selection_anchor = Some(self.issues[anchor].id);
        self.highlight(ix)
    }

    /// Adds the highlighted issue to the selection, or removes it. False
    /// if no issue is highlighted.
    pub fn toggle_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        if !self.selection.remove(&id) {
            self.selection.insert(id);
        }
        self.selection_anchor = Some(id);
        true
    }

    pub fn select_all(&mut self) {
        self.selection = self.issues.iter().map(|i| i.id).collect();
    }

    /// Empties the selection. False if it was already empty.
    pub fn clear_selection(&mut self) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        self.selection.clear();
        self.selection_anchor = None;
        true
    }
}
