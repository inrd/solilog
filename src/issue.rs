//! Domain types.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: i64,
    pub title: String,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Backlog,
    Todo,
    InProgress,
    Done,
    Blocked,
    Cancelled,
}

impl Status {
    /// All statuses, in menu order.
    pub const ALL: [Status; 6] = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::Blocked,
        Status::Done,
        Status::Cancelled,
    ];

    /// Stable identifier stored in the database.
    pub fn key(self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
            Status::Blocked => "blocked",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Backlog => "Backlog",
            Status::Todo => "Todo",
            Status::InProgress => "In Progress",
            Status::Done => "Done",
            Status::Blocked => "Blocked",
            Status::Cancelled => "Cancelled",
        }
    }

    /// Whether the issue is finished (done or cancelled).
    pub fn is_closed(self) -> bool {
        matches!(self, Status::Done | Status::Cancelled)
    }
}

impl ToSql for Status {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.key()))
    }
}

impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let key = value.as_str()?;
        Status::from_key(key)
            .ok_or_else(|| FromSqlError::Other(format!("unknown status {key:?}").into()))
    }
}
