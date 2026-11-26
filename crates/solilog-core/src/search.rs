//! Full-text search over issues: titles, descriptions, checklist items and
//! notes (`Db::search`).
//!
//! Each source has an FTS5 table (migration 23) whose rowid is the source
//! row's id, kept in sync by triggers, so every write path (GUI, CLI,
//! restores) updates it. Words match case-insensitively, ignoring accents,
//! as prefixes (`migr` finds "migration"); `"quoted words"` match as a
//! phrase. An issue matches when every word or phrase is somewhere in it,
//! not necessarily in the same field.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

use crate::Db;
use crate::filter::IssueFilter;
use crate::issue::{CompactIssue, Issue, IssueId};

/// Marks the start and end of a matched word in FTS5 highlights.
const OPEN: &str = "\u{2}";
const CLOSE: &str = "\u{3}";

/// How many words an excerpt of a description or note has around its match.
const SNIPPET_WORDS: i64 = 24;

/// What [`Db::search`] looks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Words (prefixes) and `"quoted phrases"`, all of which must match.
    pub text: String,
    /// Only issues this filter matches.
    pub filter: IssueFilter,
    /// Also search closed (done, cancelled) and archived issues. Without
    /// it, closed issues are still found when `filter.statuses` is given.
    pub include_closed: bool,
    /// At most this many issues, best first.
    pub limit: Option<usize>,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
}

/// Where a [`SearchMatch`] was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchField {
    Title,
    Description,
    ChecklistItem,
    Note,
}

impl SearchField {
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Description => "description",
            Self::ChecklistItem => "checklist_item",
            Self::Note => "note",
        }
    }
}

/// One field of an issue that matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchMatch {
    pub field: SearchField,
    /// The checklist item's or note's id; `None` for a title or description.
    pub id: Option<i64>,
    /// The matching text on one line (an excerpt of a long description or
    /// note, with `…` where it was cut), matched words in `**`.
    pub snippet: String,
    /// `snippet` without the `**` marks.
    #[serde(skip)]
    pub text: String,
    /// The matched words in `text` (byte ranges).
    #[serde(skip)]
    pub highlights: Vec<Range<usize>>,
}

impl SearchMatch {
    fn new(field: SearchField, id: Option<i64>, marked: &str) -> Self {
        let (text, highlights) = parse_marked(marked);
        let mut snippet = String::with_capacity(text.len() + 4 * highlights.len());
        let mut at = 0;
        for range in &highlights {
            snippet.push_str(&text[at..range.start]);
            snippet.push_str("**");
            snippet.push_str(&text[range.clone()]);
            snippet.push_str("**");
            at = range.end;
        }
        snippet.push_str(&text[at..]);
        Self {
            field,
            id,
            snippet,
            text,
            highlights,
        }
    }
}

/// An issue that matched, with the fields that did: title, description,
/// checklist items (in order), notes (oldest first).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub issue: CompactIssue,
    pub matches: Vec<SearchMatch>,
}

impl Db {
    /// Issues whose title, description, checklist items and notes contain
    /// every word of `query.text` (see the module docs), best matches first.
    /// Fails when the text has nothing to search for.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let terms = fts_terms(&query.text);
        anyhow::ensure!(!terms.is_empty(), "nothing to search for");

        // Issues that have every term, in any of their fields.
        let mut candidates: Option<HashSet<IssueId>> = None;
        for term in &terms {
            let found = self.issues_matching(term)?;
            candidates = Some(match candidates {
                None => found,
                Some(c) => c.intersection(&found).copied().collect(),
            });
        }
        let candidates = candidates.unwrap_or_default();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut issues: Vec<Issue> = self.list_issues(&query.filter)?;
        if query.include_closed {
            issues.extend(self.list_archived_issues(&query.filter)?);
        } else if query.filter.statuses.is_empty() {
            issues.retain(|i| !i.status.is_closed());
        }
        issues.retain(|i| candidates.contains(&i.id));
        if issues.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: HashSet<IssueId> = issues.iter().map(|i| i.id).collect();

        // The fields that have any term, for the excerpts and the ranking.
        let any = terms.join(" OR ");
        let mut found: HashMap<IssueId, (f64, Vec<SearchMatch>)> = HashMap::new();
        let mut add = |issue: IssueId, rank: f64, m: Option<SearchMatch>| {
            if wanted.contains(&issue) {
                let entry = found.entry(issue).or_insert((0.0, Vec::new()));
                entry.0 += rank;
                entry.1.extend(m);
            }
        };

        let mut stmt = self.conn.prepare_cached(
            "SELECT rowid, highlight(search_issues, 0, ?2, ?3),
                    snippet(search_issues, 1, ?2, ?3, '…', ?4),
                    bm25(search_issues, 4.0, 1.0)
             FROM search_issues WHERE search_issues MATCH ?1",
        )?;
        let rows = stmt.query_map(params![any, OPEN, CLOSE, SNIPPET_WORDS], |row| {
            Ok((
                row.get::<_, IssueId>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })?;
        for row in rows {
            let (issue, title, description, rank) = row?;
            add(issue, rank, None);
            if title.contains(OPEN) {
                add(
                    issue,
                    0.0,
                    Some(SearchMatch::new(SearchField::Title, None, &title)),
                );
            }
            if description.contains(OPEN) {
                let m = SearchMatch::new(SearchField::Description, None, &description);
                add(issue, 0.0, Some(m));
            }
        }

        for (field, sql) in [
            (
                SearchField::ChecklistItem,
                "SELECT c.issue_id, c.id, highlight(search_checklist_items, 0, ?2, ?3),
                        bm25(search_checklist_items)
                 FROM search_checklist_items
                 JOIN checklist_items c ON c.id = search_checklist_items.rowid
                 WHERE search_checklist_items MATCH ?1
                 ORDER BY c.position",
            ),
            (
                SearchField::Note,
                "SELECT n.issue_id, n.id, snippet(search_notes, 0, ?2, ?3, '…', ?4),
                        bm25(search_notes)
                 FROM search_notes
                 JOIN notes n ON n.id = search_notes.rowid
                 WHERE search_notes MATCH ?1
                 ORDER BY n.id",
            ),
        ] {
            let mut stmt = self.conn.prepare_cached(sql)?;
            // The checklist query has no `?4`.
            let rows = if field == SearchField::Note {
                stmt.query_map(params![any, OPEN, CLOSE, SNIPPET_WORDS], match_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                stmt.query_map(params![any, OPEN, CLOSE], match_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (issue, id, marked, rank) in rows {
                add(
                    issue,
                    rank,
                    Some(SearchMatch::new(field, Some(id), &marked)),
                );
            }
        }

        // bm25 is negative: lower is better. Ties: newer issues first.
        let mut hits: Vec<(f64, Issue, Vec<SearchMatch>)> = issues
            .into_iter()
            .map(|issue| {
                let (rank, matches) = found.remove(&issue.id).unwrap_or_default();
                (rank, issue, matches)
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.id.cmp(&a.1.id)));
        if let Some(limit) = query.limit {
            hits.truncate(limit);
        }
        Ok(hits
            .into_iter()
            .map(|(_, issue, matches)| SearchHit {
                issue: CompactIssue::from(issue),
                matches,
            })
            .collect())
    }

    /// Issues with `term` (an FTS5 query) in any of their fields.
    fn issues_matching(&self, term: &str) -> Result<HashSet<IssueId>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT rowid FROM search_issues WHERE search_issues MATCH ?1
                 UNION
                 SELECT c.issue_id FROM search_checklist_items
                 JOIN checklist_items c ON c.id = search_checklist_items.rowid
                 WHERE search_checklist_items MATCH ?1
                 UNION
                 SELECT n.issue_id FROM search_notes
                 JOIN notes n ON n.id = search_notes.rowid
                 WHERE search_notes MATCH ?1",
            )?
            .query_map([term], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

fn match_row(row: &rusqlite::Row) -> rusqlite::Result<(IssueId, i64, String, f64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

/// The FTS5 queries for each word (`"word"*`, a prefix) and `"quoted
/// phrase"` of `text`. Words without a letter or digit are left out: they
/// would match nothing.
fn fts_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut push = |raw: &str, phrase: bool| {
        if raw.chars().any(char::is_alphanumeric) {
            let quoted = format!("\"{}\"", raw.replace('"', "\"\""));
            terms.push(if phrase { quoted } else { format!("{quoted}*") });
        }
    };
    for (i, part) in text.split('"').enumerate() {
        // Odd parts are between quotes (an unclosed quote runs to the end).
        if i % 2 == 1 {
            push(part.trim(), true);
        } else {
            for word in part.split_whitespace() {
                push(word, false);
            }
        }
    }
    terms
}

/// Text marked with [`OPEN`] / [`CLOSE`], as one line (whitespace runs
/// become one space), and the marked ranges in it.
fn parse_marked(marked: &str) -> (String, Vec<Range<usize>>) {
    let mut text = String::with_capacity(marked.len());
    let mut highlights = Vec::new();
    let mut start = None;
    let mut space = false;
    for c in marked.chars() {
        match c {
            '\u{2}' => {
                if space && !text.is_empty() {
                    text.push(' ');
                }
                space = false;
                start = Some(text.len());
            }
            '\u{3}' => {
                if let Some(s) = start.take()
                    && s < text.len()
                {
                    highlights.push(s..text.len());
                }
            }
            c if c.is_whitespace() => space = true,
            c => {
                if space && !text.is_empty() {
                    text.push(' ');
                }
                space = false;
                text.push(c);
            }
        }
    }
    (text, highlights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueChange, IssueEdit, NewIssue, Status};

    fn ids(hits: &[SearchHit]) -> Vec<IssueId> {
        hits.iter().map(|h| h.issue.id).collect()
    }

    #[test]
    fn terms() {
        assert_eq!(fts_terms("sqlite wal"), ["\"sqlite\"*", "\"wal\"*"]);
        assert_eq!(
            fts_terms(r#"say "hello world" - "#),
            ["\"say\"*", "\"hello world\""]
        );
        assert_eq!(fts_terms("SOL-12"), ["\"SOL-12\"*"]);
        assert_eq!(fts_terms("a\"b"), ["\"a\"*", "\"b\""]);
        assert!(fts_terms(" -- * ").is_empty());
    }

    #[test]
    fn marked_text() {
        let (text, ranges) = parse_marked("one \u{2}two\u{3}\n\n  three \u{2}four\u{3}");
        assert_eq!(text, "one two three four");
        assert_eq!(ranges, [4..7, 14..18]);
        let m = SearchMatch::new(SearchField::Note, Some(1), "a \u{2}b\u{3} c");
        assert_eq!(m.snippet, "a **b** c");
    }

    #[test]
    fn searches_every_field_and_ranks_titles_first() {
        let db = Db::open_in_memory().unwrap();
        let titled = db.create_issue(&NewIssue::new("SQLite WAL mode")).unwrap();
        let described = db
            .create_issue(
                &NewIssue::new("Concurrency").description(
                    "The GUI and the CLI share the database.\n\nUse sqlite in WAL mode.",
                ),
            )
            .unwrap();
        let checked = db
            .create_issue(&NewIssue::new("Setup").checklist(["enable sqlite wal"]))
            .unwrap();
        let noted = db.create_issue(&NewIssue::new("Locks")).unwrap();
        let note = db
            .add_note(noted.id, "Tried sqlite WAL: fixed the busy errors")
            .unwrap();
        // Terms in different fields of one issue: it still matches.
        let split = db
            .create_issue(&NewIssue::new("sqlite tuning").description("checkpoint the wal"))
            .unwrap();
        db.create_issue(&NewIssue::new("Unrelated").description("sqlite only"))
            .unwrap();

        let hits = db.search(&SearchQuery::new("sqlite wal")).unwrap();
        assert_eq!(hits.len(), 5);
        assert_eq!(hits[0].issue.id, titled.id);
        let found: HashSet<_> = ids(&hits).into_iter().collect();
        for id in [described.id, checked.id, noted.id, split.id] {
            assert!(found.contains(&id));
        }

        let by_id = |id| hits.iter().find(|h| h.issue.id == id).unwrap();
        assert_eq!(by_id(titled.id).matches[0].field, SearchField::Title);
        assert_eq!(
            by_id(titled.id).matches[0].snippet,
            "**SQLite** **WAL** mode"
        );
        let desc = &by_id(described.id).matches[0];
        assert_eq!(desc.field, SearchField::Description);
        assert!(desc.snippet.contains("Use **sqlite** in **WAL** mode."));
        assert!(!desc.text.contains('\n'));
        let item = &by_id(checked.id).matches[0];
        assert_eq!(item.field, SearchField::ChecklistItem);
        assert_eq!(item.id, Some(checked.checklist[0].id.0));
        let n = &by_id(noted.id).matches[0];
        assert_eq!((n.field, n.id), (SearchField::Note, Some(note.id.0)));
        let fields: Vec<_> = by_id(split.id).matches.iter().map(|m| m.field).collect();
        assert_eq!(fields, [SearchField::Title, SearchField::Description]);

        // Prefixes, case and accents.
        assert_eq!(
            ids(&db.search(&SearchQuery::new("SQLI MOD")).unwrap())[0],
            titled.id
        );
        let accented = db.create_issue(&NewIssue::new("Café crème")).unwrap();
        assert_eq!(
            ids(&db.search(&SearchQuery::new("cafe creme")).unwrap()),
            [accented.id]
        );
        // A phrase must match as written.
        let phrase = db.search(&SearchQuery::new("\"wal mode\"")).unwrap();
        assert_eq!(ids(&phrase).len(), 2);
        assert!(
            db.search(&SearchQuery::new("nothing-like-this"))
                .unwrap()
                .is_empty()
        );
        assert!(db.search(&SearchQuery::new("  -  ")).is_err());
        let limited = db
            .search(&SearchQuery {
                limit: Some(2),
                ..SearchQuery::new("sqlite")
            })
            .unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn closed_and_archived_issues_only_when_asked() {
        let db = Db::open_in_memory().unwrap();
        let open = db.create_issue(&NewIssue::new("reverb tail")).unwrap();
        let done = db
            .create_issue(&NewIssue::new("reverb click").status(Status::Done))
            .unwrap();
        let archived = db.create_issue(&NewIssue::new("reverb old")).unwrap();
        db.set_archived(archived.id, true).unwrap();

        let search = |query: SearchQuery| {
            let mut found = ids(&db.search(&query).unwrap());
            found.sort();
            found
        };
        assert_eq!(search(SearchQuery::new("reverb")), [open.id]);
        assert_eq!(
            search(SearchQuery {
                include_closed: true,
                ..SearchQuery::new("reverb")
            }),
            [open.id, done.id, archived.id]
        );
        // Asking for a closed status finds closed issues.
        assert_eq!(
            search(SearchQuery {
                filter: IssueFilter {
                    statuses: vec![Status::Done],
                    ..Default::default()
                },
                ..SearchQuery::new("reverb")
            }),
            [done.id]
        );
    }

    #[test]
    fn filters_apply() {
        let db = Db::open_in_memory().unwrap();
        let acme = db
            .create_issue(&NewIssue::new("reverb tail").labels(["acme"]))
            .unwrap();
        db.create_issue(&NewIssue::new("reverb click")).unwrap();
        let hits = db
            .search(&SearchQuery {
                filter: IssueFilter {
                    labels: vec!["acme".into()],
                    ..Default::default()
                },
                ..SearchQuery::new("reverb")
            })
            .unwrap();
        assert_eq!(ids(&hits), [acme.id]);
    }

    #[test]
    fn index_follows_every_write() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("alpha").checklist(["bravo"]))
            .unwrap();
        let item = issue.checklist[0].id;
        let note = db.add_note(issue.id, "charlie").unwrap();
        let found = |q: &str| ids(&db.search(&SearchQuery::new(q)).unwrap());
        for word in ["alpha", "bravo", "charlie"] {
            assert_eq!(found(word), [issue.id], "{word}");
        }

        db.edit_issue(
            issue.id,
            &IssueEdit {
                title: Some("delta".into()),
                description: Some("echo".into()),
                ..Default::default()
            },
        )
        .unwrap();
        db.edit_checklist_item(item, "foxtrot").unwrap();
        assert!(found("alpha").is_empty());
        assert!(found("bravo").is_empty());
        for word in ["delta", "echo", "foxtrot"] {
            assert_eq!(found(word), [issue.id], "{word}");
        }

        db.delete_note(note.id).unwrap();
        assert!(found("charlie").is_empty());
        db.delete_checklist_item(item).unwrap();
        assert!(found("foxtrot").is_empty());

        // Status changes don't touch the index; deleting an issue empties it.
        db.update_issue(issue.id, &IssueChange::SetStatus(Status::InProgress))
            .unwrap();
        assert_eq!(found("delta"), [issue.id]);
        let other = db
            .create_issue(&NewIssue::new("golf").checklist(["hotel"]))
            .unwrap();
        db.add_note(other.id, "india").unwrap();
        db.delete_issue(other.id).unwrap();
        for table in ["search_issues", "search_checklist_items", "search_notes"] {
            let rows: i64 = db
                .conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            let expected = i64::from(table == "search_issues");
            assert_eq!(rows, expected, "{table}");
        }
    }

    #[test]
    fn search_triggers_exist() {
        // Rebuilding a table drops its triggers: a later migration that
        // rebuilds one of these tables must create them again.
        let db = Db::open_in_memory().unwrap();
        let triggers: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'search_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(triggers, 9);
    }
}
