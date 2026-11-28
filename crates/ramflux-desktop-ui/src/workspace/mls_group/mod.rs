// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS groups on the desktop (MLS GUI batch M2): the list, creating a
//! group, invites, joining with a link, members, group info and the
//! conversation chrome.
//!
//! Every view reads the account's store and asks through UI events; the
//! store holds the join owner, the send owner and the timelines, so closing
//! a sheet never loses an outcome (S4). Nothing here sends anything by
//! itself: each call is one explicit tap (S3), and a post that was not sent
//! is sent again only by its Retry.
//!
//! - [`section`]: the group list (the Groups route while on the mock).
//! - [`create`]: the new group dialog.
//! - [`join`]: the join sheet.
//! - [`invite`]: the invite panel.
//! - [`members`]: the member list.
//! - [`info`]: the group info panel.
//! - [`confirm`]: the confirmation of revoking, deleting and leaving.
//! - [`chrome`]: the conversation header, the read-only banner and the
//!   mention strip.

pub mod banners;
pub mod chrome;
pub mod confirm;
pub mod create;
pub mod info;
pub mod invite;
pub mod join;
pub mod members;
pub mod requests;
pub mod section;
pub mod settings;

use gpui_kit::SharedString;
use ramflux_desktop_core::mls_group::{Author, MemberRow, PrivacyDelay, Quota};
use ramflux_desktop_core::{Store, Timestamp};

use crate::i18n::{self, tr};

pub use chrome::MentionPick;

/// The mentions picked for `group_id`, in the order picked. They live in
/// the account's store, so a lock, a removal or an account switch never
/// shows or sends another account's picks.
pub fn picked_mentions(store: &Store, group_id: &str) -> Vec<(String, MentionPick)> {
    store
        .mls_mentions(group_id)
        .into_iter()
        .map(|(anchor, name)| (anchor, MentionPick { name: name.into() }))
        .collect()
}

/// A name as shown: "You", the display name (bidi-isolated), or "Member".
pub fn name_text(display: Option<&str>, is_me: bool) -> SharedString {
    if is_me {
        tr("group.members.you")
    } else {
        match display.filter(|d| !d.is_empty()) {
            Some(name) => i18n::bidi::isolate(&display_name(name)).into(),
            None => tr("group.member.unnamed"),
        }
    }
}

/// A display name from another client: long names are cut (S10).
fn display_name(name: &str) -> String {
    const LIMIT: usize = 64;
    if name.chars().count() > LIMIT {
        let cut: String = name.chars().take(LIMIT).collect();
        format!("{cut}\u{2026}")
    } else {
        name.to_owned()
    }
}

/// The name of an author.
pub fn author_name(author: &Author) -> SharedString {
    name_text(author.display.as_deref(), author.is_me)
}

/// The name of a member row.
pub fn member_name(row: &MemberRow) -> SharedString {
    name_text(row.display.as_deref(), row.is_me)
}

/// A group name from the SDK, cut for display (S10) and isolated.
pub fn group_name(name: &str) -> SharedString {
    i18n::bidi::isolate(&display_name(name)).into()
}

/// A time in the locale's date and time pattern.
pub fn when(at: Timestamp) -> String {
    i18n::format::date_time(at)
}

/// Whether a quota is shown: on the first day, or when what is left is a
/// fifth of the limit or less (at least one). Showing it changes nothing.
pub fn quota_due(quota: &Quota) -> bool {
    quota.first_day || quota.remaining <= (quota.limit / 5).max(1)
}

/// The line of the post quota (the composer area).
pub fn post_quota_text(quota: &Quota) -> SharedString {
    let args = [("left", quota.remaining.to_string()), ("limit", quota.limit.to_string())];
    let args = [(args[0].0, args[0].1.as_str()), (args[1].0, args[1].1.as_str())];
    i18n::tr_args(
        if quota.first_day { "group.quota.post_first_day" } else { "group.quota.post" },
        &args,
    )
}

/// The line of the commit quota (the group info).
pub fn commit_quota_text(quota: &Quota) -> SharedString {
    let left = quota.remaining.to_string();
    let limit = quota.limit.to_string();
    i18n::tr_args("group.quota.commit", &[("left", &left), ("limit", &limit)])
}

/// When a post sent now goes out, while that is after `now` (the privacy
/// delay with a time in the future); a time in the past reads as no time.
pub fn privacy_eta(delay: &PrivacyDelay, now: Timestamp) -> Option<Timestamp> {
    match delay {
        PrivacyDelay::Waiting(at) if at.0 > now.0 => Some(*at),
        _ => None,
    }
}

/// Whether the privacy delay is armed (a time in the past counts as armed).
pub fn privacy_armed(delay: &PrivacyDelay) -> bool {
    !matches!(delay, PrivacyDelay::None)
}

/// A wait as the join and rate texts show it: minutes under an hour, else
/// hours (at least one of the unit).
pub fn wait_text(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60).max(1);
    if minutes < 60 {
        i18n::tr_args("format.duration.minutes", &[("count", &minutes.to_string())]).to_string()
    } else {
        let hours = minutes.div_ceil(60);
        i18n::tr_args("format.duration.hours", &[("count", &hours.to_string())]).to_string()
    }
}
