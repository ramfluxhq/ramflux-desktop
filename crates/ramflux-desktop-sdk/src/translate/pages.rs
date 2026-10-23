// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Collecting the pages of a paged list into one complete listing.
//!
//! A listing starts with a first page (no cursor) and ends with a page whose
//! `next_cursor` is null; only then is it translated, so a row missing from
//! a complete listing means the row is gone. A first page restarts the
//! collection. A later page that belongs to no collection in progress (its
//! listing was restarted or already completed) is dropped and not followed,
//! so two overlapping reads never combine into a short listing. Every page
//! is a fresh daemon snapshot; rows are keyed by id, a later page wins.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::plan::fetch::{Fetch, Paged};

/// Pages one listing may span before it is abandoned (a daemon that keeps
/// returning cursors).
pub const MAX_PAGES: usize = 10_000;

/// What one page means.
#[derive(Debug, PartialEq)]
pub enum Paging {
    /// More pages follow: read this next.
    More(Fetch),
    /// The listing is complete: every row, in daemon order.
    Complete(Vec<Value>),
    /// The page belongs to no listing in progress, or the listing was
    /// abandoned.
    Dropped,
}

#[derive(Debug, Default)]
struct Collecting {
    rows: Vec<Value>,
    pages: usize,
}

/// Listings in progress.
#[derive(Debug, Default)]
pub struct Pages {
    collecting: BTreeMap<Paged, Collecting>,
}

/// The id a row is keyed by.
fn row_id(row: &Value) -> Option<&str> {
    ["id", "group_id", "approval_id"].into_iter().find_map(|f| row.get(f).and_then(Value::as_str))
}

impl Pages {
    /// Adds the page of `paged` read with `cursor`.
    pub fn page(&mut self, paged: Paged, cursor: Option<&str>, value: &Value) -> Paging {
        if cursor.is_none() {
            self.collecting.insert(paged, Collecting::default());
        }
        let Some(collecting) = self.collecting.get_mut(&paged) else { return Paging::Dropped };
        let (_, _, field) = paged.method();
        let rows = value.get(field).and_then(Value::as_array).cloned().unwrap_or_default();
        for row in rows {
            match row_id(&row).map(str::to_owned) {
                Some(id) => {
                    collecting.rows.retain(|r| row_id(r) != Some(id.as_str()));
                    collecting.rows.push(row);
                }
                None => collecting.rows.push(row),
            }
        }
        collecting.pages += 1;
        match value.get("next_cursor").and_then(Value::as_str) {
            Some(next) if collecting.pages < MAX_PAGES => {
                Paging::More(Fetch::Page(paged, Some(next.to_owned())))
            }
            Some(_) => {
                self.collecting.remove(&paged);
                Paging::Dropped
            }
            None => {
                let done = self.collecting.remove(&paged).unwrap_or_default();
                Paging::Complete(done.rows)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn pages_complete_on_a_null_cursor_and_strays_are_dropped() {
        let mut pages = Pages::default();
        let first = json!({ "objects": [{ "id": "a" }, { "id": "b" }], "next_cursor": "c1" });
        assert_eq!(
            pages.page(Paged::Objects, None, &first),
            Paging::More(Fetch::Page(Paged::Objects, Some("c1".to_owned())))
        );
        // A short page with a cursor (the response budget) is followed too.
        let budget = json!({ "objects": [{ "id": "c" }], "next_cursor": "c2" });
        assert!(matches!(pages.page(Paged::Objects, Some("c1"), &budget), Paging::More(_)));
        let last = json!({ "objects": [{ "id": "d" }], "next_cursor": null });
        let Paging::Complete(rows) = pages.page(Paged::Objects, Some("c2"), &last) else {
            panic!("complete expected");
        };
        let ids: Vec<&str> = rows.iter().filter_map(row_id).collect();
        assert_eq!(ids, ["a", "b", "c", "d"]);
        // A later page of a listing that already completed is dropped.
        assert_eq!(pages.page(Paged::Objects, Some("c2"), &last), Paging::Dropped);
    }

    #[test]
    fn a_first_page_restarts_the_listing() {
        let mut pages = Pages::default();
        let page = json!({ "groups": [{ "group_id": "g1" }], "next_cursor": "x" });
        let _ = pages.page(Paged::Groups, None, &page);
        let again = json!({ "groups": [{ "group_id": "g2" }], "next_cursor": null });
        let Paging::Complete(rows) = pages.page(Paged::Groups, None, &again) else {
            panic!("complete expected");
        };
        assert_eq!(rows.len(), 1, "the earlier collection was discarded");
    }
}
