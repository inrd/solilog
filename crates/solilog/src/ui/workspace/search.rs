//! Quick search (`workspace::Search`): a picker that searches every issue,
//! closed and archived ones included (titles, descriptions, checklist items,
//! notes; `Db::search`) as you type. Choosing a result opens it.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Action as _, AppContext as _, Context, Window, px};
use solilog_core::{IssueId, SearchField, SearchHit, SearchQuery};

use super::{Modal, Workspace};
use crate::ui::actions::{issue, workspace::Search};
use crate::ui::picker::{Picker, PickerItem};
use crate::ui::status_icon::status_icon;
use crate::ui::theme::{self, Theme};

/// How many issues the picker lists.
const LIMIT: usize = 50;

/// "SOL-7  Title", with the first match that isn't the title below it.
fn hit_item(hit: &SearchHit, t: Rc<Theme>) -> PickerItem {
    let status = hit.issue.status;
    let item = PickerItem::new(format!("{}  {}", hit.issue.id, hit.issue.title))
        .icon(move || status_icon(status, &t))
        .hint(hit.issue.archived.then_some("archived"));
    let Some(m) = hit.matches.iter().find(|m| m.field != SearchField::Title) else {
        return item;
    };
    let prefix = match m.field {
        SearchField::Title | SearchField::Description => "",
        SearchField::ChecklistItem => "Checklist: ",
        SearchField::Note => "Note: ",
    };
    let bold = m
        .highlights
        .iter()
        .map(|r| r.start + prefix.len()..r.end + prefix.len())
        .collect();
    item.detail(format!("{prefix}{}", m.text), bold)
}

impl Workspace {
    pub(super) fn on_search(&mut self, _: &Search, window: &mut Window, cx: &mut Context<Self>) {
        // Edits not saved yet should be found.
        self.save_detail(false, cx);
        let ids: Rc<RefCell<Vec<IssueId>>> = Rc::default();
        let workspace = cx.weak_entity();
        let found = ids.clone();
        let picker = cx.new(|cx| {
            Picker::new("Search all issues…", Vec::new(), 0, cx)
                .width(px(640.))
                .searching(
                    "Titles, descriptions, checklists and notes, closed and archived issues too",
                    move |query, cx| {
                        let t = theme::current(cx);
                        let hits = workspace
                            .upgrade()
                            .and_then(|w| {
                                let query = SearchQuery {
                                    include_closed: true,
                                    limit: Some(LIMIT),
                                    ..SearchQuery::new(query)
                                };
                                // An empty query is an error: nothing to show.
                                w.read(cx).db.search(&query).ok()
                            })
                            .unwrap_or_default();
                        *found.borrow_mut() = hits.iter().map(|h| h.issue.id).collect();
                        hits.iter().map(|h| hit_item(h, t.clone())).collect()
                    },
                )
        });
        self.open_modal(picker, |picker| Modal::Search { picker, ids }, window, cx);
    }

    /// Opens the issue chosen in the search picker.
    pub(super) fn open_search_result(
        &mut self,
        id: IssueId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.dispatch_action(issue::Open { id: Some(id) }.boxed_clone(), cx);
    }
}
