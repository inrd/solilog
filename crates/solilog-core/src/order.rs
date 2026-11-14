//! User-defined order of projects and labels: an integer `position` column,
//! ascending. New rows get a position after every existing one.

use anyhow::{Context as _, Result};
use rusqlite::params;

use crate::Db;

/// Where to move a project or label, relative to another one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    Before(String),
    After(String),
}

impl Placement {
    pub fn anchor(&self) -> &str {
        match self {
            Placement::Before(name) | Placement::After(name) => name,
        }
    }

    /// Builds a placement from `--before`/`--after` style options; exactly
    /// one must be given.
    pub fn from_options(before: Option<String>, after: Option<String>) -> Result<Self> {
        match (before, after) {
            (Some(before), None) => Ok(Placement::Before(before)),
            (None, Some(after)) => Ok(Placement::After(after)),
            _ => anyhow::bail!("give exactly one of `before` and `after`"),
        }
    }
}

/// Tables with a `position` column.
#[derive(Clone, Copy)]
pub(crate) enum Ordered {
    Projects,
    Labels,
}

impl Ordered {
    fn table(self) -> &'static str {
        match self {
            Ordered::Projects => "projects",
            Ordered::Labels => "labels",
        }
    }
}

impl Db {
    /// A position after every existing row of the table.
    pub(crate) fn next_position(&self, table: Ordered) -> Result<i64> {
        Ok(self.conn.query_row(
            &format!(
                "SELECT coalesce(max(position), 0) + 1 FROM {}",
                table.table()
            ),
            [],
            |row| row.get(0),
        )?)
    }

    /// Moves row `id` before or after row `anchor` within `order` (the ids
    /// of the rows ranked together, in their current order), and renumbers
    /// them. Moving a row next to itself does nothing. Call inside a
    /// transaction.
    pub(crate) fn place(
        &self,
        table: Ordered,
        mut order: Vec<i64>,
        id: i64,
        anchor: i64,
        placement: &Placement,
    ) -> Result<()> {
        if anchor == id {
            return Ok(());
        }
        order.retain(|&row| row != id);
        let ix = order
            .iter()
            .position(|&row| row == anchor)
            .context("row vanished while moving")?;
        let after = matches!(placement, Placement::After(_));
        order.insert(ix + usize::from(after), id);
        let mut update = self.conn.prepare_cached(&format!(
            "UPDATE {} SET position = ?2 WHERE id = ?1",
            table.table()
        ))?;
        for (position, row) in order.iter().enumerate() {
            update.execute(params![row, position as i64 + 1])?;
        }
        Ok(())
    }
}
