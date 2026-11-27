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

#[cfg(test)]
mod tests {
    use solilog_core::{Db, NewIssue};

    use super::*;
    use Status::{Done, InProgress, Todo};

    /// One issue per status, in that order, with ids 1, 2, 3…
    fn issues(statuses: &[Status]) -> Vec<Issue> {
        let db = Db::open_in_memory().unwrap();
        statuses
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                db.create_issue(&NewIssue::new(format!("issue {}", i + 1)).status(s))
                    .unwrap()
            })
            .collect()
    }

    fn ids(issues: &[Issue]) -> Vec<i64> {
        issues.iter().map(|i| i.id.0).collect()
    }

    fn id(n: i64) -> IssueId {
        IssueId(n)
    }

    fn selection(list: &ListState) -> Vec<i64> {
        let mut ids: Vec<i64> = list.selection.iter().map(|id| id.0).collect();
        ids.sort();
        ids
    }

    /// Lists five `todo` issues, 1 to 5, the first highlighted.
    fn five() -> ListState {
        let mut list = ListState::default();
        list.set_issues(issues(&[Todo; 5]), None);
        list
    }

    #[test]
    fn sections_follow_list_order_keeping_order_within_a_status() {
        let mut list = ListState::default();
        list.set_issues(issues(&[Todo, InProgress, Todo, Done]), None);
        assert_eq!(ids(&list.issues), [2, 1, 3, 4]);
        assert_eq!(list.sections, [(InProgress, 1), (Todo, 2), (Done, 1)]);
    }

    #[test]
    fn collapsed_sections_keep_their_header_but_not_their_issues() {
        let mut list = ListState {
            collapsed_statuses: vec![Todo],
            ..ListState::default()
        };
        list.set_issues(issues(&[Todo, InProgress, Todo, Done]), None);
        assert_eq!(ids(&list.issues), [2, 4]);
        assert_eq!(list.sections, [(InProgress, 1), (Todo, 2), (Done, 1)]);

        // Rows come after their own header and every earlier one, the
        // collapsed section's included.
        assert_eq!(list.child_index(0), 1);
        assert_eq!(list.child_index(1), 4);
        assert_eq!(list.child_index(7), 7, "past the end: unchanged");
    }

    #[test]
    fn the_first_issue_is_highlighted_on_first_load() {
        let list = five();
        assert_eq!(list.selected, Some(id(1)));

        let mut empty = ListState::default();
        empty.set_issues(Vec::new(), None);
        assert_eq!(empty.selected, None);
        assert_eq!(empty.last_index(), 0);
        assert!(!empty.select(0));
    }

    #[test]
    fn reloading_keeps_the_highlighted_issue_wherever_it_moves() {
        let mut list = ListState::default();
        let all = issues(&[Todo, Todo, Todo]);
        list.set_issues(all.clone(), None);
        assert!(list.select(2));
        let prev = list.selected_index();

        // Issue 3 is now in progress: listed first.
        let mut moved = all;
        moved[2].status = InProgress;
        list.set_issues(moved, prev);
        assert_eq!(list.selected, Some(id(3)));
        assert_eq!(list.selected_index(), Some(0));
        assert_eq!(list.highlighted().map(|i| i.id), Some(id(3)));
    }

    #[test]
    fn reloading_without_the_highlighted_issue_keeps_its_row() {
        let mut list = five();
        let all = list.issues.clone();
        assert!(list.select(1));

        let without = |gone: i64| {
            all.iter()
                .filter(|i| i.id.0 != gone)
                .cloned()
                .collect::<Vec<_>>()
        };
        let prev = list.selected_index();
        list.set_issues(without(2), prev);
        assert_eq!(list.selected, Some(id(3)), "the next issue takes its row");

        // The last row: the new last one.
        list.set_issues(all.clone(), list.selected_index());
        assert!(list.select(4));
        let prev = list.selected_index();
        list.set_issues(without(5), prev);
        assert_eq!(list.selected, Some(id(4)));

        // Nothing left.
        list.set_issues(Vec::new(), list.selected_index());
        assert_eq!(list.selected, None);
    }

    #[test]
    fn reloading_drops_selected_issues_that_are_no_longer_listed() {
        let mut list = five();
        let all = list.issues.clone();
        list.select(2);
        list.toggle_selected();
        list.select(0);
        list.toggle_selected();
        assert_eq!(selection(&list), [1, 3]);
        assert_eq!(list.selection_anchor, Some(id(1)));

        // Issue 1 leaves the list, the anchor with it.
        list.set_issues(all[1..].to_vec(), list.selected_index());
        assert_eq!(selection(&list), [3]);
        assert_eq!(list.selection_anchor, None);

        // A collapsed section's issues aren't listed either.
        let mut all_done = all;
        all_done[2].status = Done;
        list.collapsed_statuses = vec![Done];
        list.set_issues(all_done, list.selected_index());
        assert!(list.selection.is_empty());
    }

    #[test]
    fn extending_selects_from_the_anchor_replacing_the_previous_range() {
        let mut list = five();
        assert!(list.select(1));
        assert!(list.extend_selection_to(3));
        assert_eq!(selection(&list), [2, 3, 4]);
        assert_eq!(list.selected, Some(id(4)));
        assert_eq!(
            list.selection_anchor,
            Some(id(2)),
            "where the range started"
        );

        // Back past the anchor: the range now runs up from it.
        assert!(list.extend_selection_to(0));
        assert_eq!(selection(&list), [1, 2]);
        assert_eq!(list.selected, Some(id(1)));

        // Past the end: up to the last issue.
        assert!(list.extend_selection_to(99));
        assert_eq!(selection(&list), [2, 3, 4, 5]);
        assert_eq!(list.selected, Some(id(5)));
    }

    #[test]
    fn extending_keeps_issues_picked_outside_the_range() {
        let mut list = five();
        list.select(4);
        assert!(list.toggle_selected());
        assert_eq!(list.selection_anchor, Some(id(5)));

        // Moving the highlight without extending starts a new range there.
        list.select(0);
        assert_eq!(list.selection_anchor, None);
        list.extend_selection_to(1);
        assert_eq!(selection(&list), [1, 2, 5]);
    }

    #[test]
    fn extending_without_a_highlight_just_highlights() {
        let mut list = five();
        list.selected = None;
        assert!(list.extend_selection_to(2));
        assert_eq!(list.selected, Some(id(3)));
        assert!(list.selection.is_empty());
        assert_eq!(list.selection_anchor, None);
    }

    #[test]
    fn toggling_select_all_and_clearing() {
        let mut list = five();
        list.select(1);
        assert!(list.toggle_selected());
        assert_eq!(selection(&list), [2]);
        assert!(list.toggle_selected());
        assert!(list.selection.is_empty());

        list.select_all();
        assert_eq!(selection(&list), [1, 2, 3, 4, 5]);
        assert!(list.clear_selection());
        assert!(list.selection.is_empty());
        assert_eq!(list.selection_anchor, None);
        assert!(!list.clear_selection(), "nothing left to clear");

        list.selected = None;
        assert!(!list.toggle_selected(), "no highlighted issue");
    }
}
