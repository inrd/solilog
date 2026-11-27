//! Adding and removing issue dependencies (`issue::AddDependency`,
//! `issue::RemoveDependency`), with an issue picker when no issue is given.

use std::rc::Rc;

use gpui::{AppContext as _, Context, Window};
use solilog_core::{Issue, IssueChange, IssueId, Status};

use super::{Modal, Workspace, of_issues};
use crate::ui::actions::issue::{AddDependency, RemoveDependency};
use crate::ui::picker::{Picker, PickerItem};
use crate::ui::status_icon::status_icon;
use crate::ui::theme::{self, Theme};

/// "SOL-7  Title", as listed in pickers.
fn issue_item(id: IssueId, title: &str, status: Status, t: Rc<Theme>) -> PickerItem {
    PickerItem::new(format!("{id}  {title}")).icon(move || status_icon(status, &t))
}

impl Workspace {
    /// The title and status of an issue, from the cache when it is there.
    pub(super) fn issue_summary(&self, id: IssueId) -> Option<(String, Status)> {
        self.all_issues
            .iter()
            .chain(&self.archive.issues)
            .find(|i| i.id == id)
            .map(|i| (i.title.clone(), i.status))
            .or_else(|| {
                let issue = self.db.get_issue(id).ok()?;
                Some((issue.title, issue.status))
            })
    }

    pub(super) fn on_add_dependency(
        &mut self,
        action: &AddDependency,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return;
        }
        let ids: Vec<_> = issues.iter().map(|i| i.id).collect();
        match action.on {
            Some(on) => self.update_issues(&ids, IssueChange::AddDependencies(vec![on]), cx),
            None => {
                // Every listed issue but those that would be refused (the
                // targets and the issues depending on them, which would
                // make a cycle) and those they all depend on already; open
                // ones first.
                let Some(excluded) = self.report(self.db.cannot_depend_on(&ids), cx) else {
                    return;
                };
                let mut candidates: Vec<&Issue> = self
                    .all_issues
                    .iter()
                    .filter(|i| excluded.binary_search(&i.id).is_err())
                    .filter(|i| !issues.iter().all(|t| t.depends_on.contains(&i.id)))
                    .collect();
                candidates.sort_by_key(|i| i.status.is_closed());
                let t = theme::current(cx);
                let items = candidates
                    .iter()
                    .map(|i| issue_item(i.id, &i.title, i.status, t.clone()))
                    .collect();
                let on = candidates.iter().map(|i| i.id).collect();
                self.open_dependency_picker(true, &issues, items, on, window, cx);
            }
        }
    }

    pub(super) fn on_remove_dependency(
        &mut self,
        action: &RemoveDependency,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return;
        }
        let ids: Vec<_> = issues.iter().map(|i| i.id).collect();
        match action.on {
            Some(on) => self.update_issues(&ids, IssueChange::RemoveDependencies(vec![on]), cx),
            None => {
                let mut on: Vec<IssueId> = issues
                    .iter()
                    .flat_map(|i| i.depends_on.iter().copied())
                    .collect();
                on.sort();
                on.dedup();
                if on.is_empty() {
                    self.footer.error = Some(match issues.as_slice() {
                        [issue] => format!("{} depends on no issue", issue.id),
                        _ => "The selected issues depend on no issue".into(),
                    });
                    cx.notify();
                    return;
                }
                let t = theme::current(cx);
                let items = on
                    .iter()
                    .map(|&id| {
                        let (title, status) = self
                            .issue_summary(id)
                            .unwrap_or_else(|| (String::new(), Status::Todo));
                        issue_item(id, &title, status, t.clone())
                    })
                    .collect();
                self.open_dependency_picker(false, &issues, items, on, window, cx);
            }
        }
    }

    fn open_dependency_picker(
        &mut self,
        add: bool,
        issues: &[Issue],
        items: Vec<PickerItem>,
        on: Vec<IssueId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let verb = if add {
            "Add dependency"
        } else {
            "Remove dependency"
        };
        let placeholder = format!("{verb}{}…", of_issues(issues));
        let picker = cx.new(|cx| Picker::new(&placeholder, items, 0, cx));
        let issue_ids = issues.iter().map(|i| i.id).collect();
        self.open_modal(
            picker,
            |picker| Modal::Dependency {
                picker,
                issue_ids,
                add,
                on,
            },
            window,
            cx,
        );
    }

    /// Applies the issue picked in a dependency picker.
    pub(super) fn apply_dependency(
        &mut self,
        add: bool,
        issue_ids: &[IssueId],
        on: IssueId,
        cx: &mut Context<Self>,
    ) {
        let change = if add {
            IssueChange::AddDependencies(vec![on])
        } else {
            IssueChange::RemoveDependencies(vec![on])
        };
        self.update_issues(issue_ids, change, cx);
    }
}
