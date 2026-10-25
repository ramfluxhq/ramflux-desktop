// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the command palette offers, built from the store: every context (the
//! full conversation list is reachable only here), contacts, objects,
//! message history, and actions on the current context and the workspace.

use gpui_kit::SharedString;
use gpui_kit::component::IconName;
use ramflux_desktop_core::{
    AccountId, ContextId, ContextKind, MomentId, MomentKind, ObjectId, ParticipantId, Store,
    selectors,
};

use crate::i18n::{tr, tr_args};
use crate::workspace::{DockTool, TIMERS, timer_label};

/// Longest history snippet shown in a row.
const SNIPPET: usize = 60;

/// What choosing an entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteChoice {
    /// Show a context.
    Context(ContextId),
    /// Show a context and focus a moment in it.
    Moment(ContextId, MomentId),
    /// Show the objects dock.
    Object(ObjectId),
    /// Show the direct context with a contact, else a new-conversation
    /// draft with them.
    Contact(ParticipantId),
    /// Pin or unpin a context (an intent).
    Pin(ContextId, bool),
    /// Mute or unmute a context (an intent).
    Mute(ContextId, bool),
    /// Set a context's disappearing timer (an intent).
    Disappearing(ContextId, Option<u32>),
    /// Render another account.
    SwitchAccount(AccountId),
    /// Open settings.
    OpenSettings,
    /// Open a dock tool.
    OpenDock(DockTool),
    /// Open the contacts dock at its add-contact field.
    AddContact,
    /// Open the new-group form.
    NewGroup,
    /// Address the composer's next request to an AI participant of the
    /// shown conversation (view-local; the request itself is an intent).
    AskAgent(ParticipantId),
    /// Move to the next item that needs the user.
    NeedsMe,
    /// Switch between the light and dark theme.
    ToggleTheme,
}

/// One palette row.
#[derive(Clone)]
pub struct Entry {
    /// Visible label.
    pub label: SharedString,
    /// Extra search terms.
    pub keywords: Vec<String>,
    /// Leading icon.
    pub icon: IconName,
    /// What choosing it does.
    pub choice: PaletteChoice,
}

/// A labeled group of rows.
#[derive(Clone)]
pub struct Group {
    /// Heading.
    pub label: SharedString,
    /// Rows.
    pub entries: Vec<Entry>,
}

fn entry(label: impl Into<SharedString>, icon: IconName, choice: PaletteChoice) -> Entry {
    Entry { label: label.into(), keywords: Vec::new(), icon, choice }
}

/// Builds the palette groups for `store`, with actions on `current`.
pub fn groups(
    store: &Store,
    current: Option<&ContextId>,
    accounts: &[(AccountId, String)],
    active: Option<&AccountId>,
) -> Vec<Group> {
    let mut groups = vec![contexts(store), actions(store, current, accounts, active)];
    let contacts: Vec<Entry> = selectors::contacts(store)
        .into_iter()
        .map(|p| {
            entry(p.display.clone(), IconName::CircleUser, PaletteChoice::Contact(p.id.clone()))
        })
        .collect();
    let objects: Vec<Entry> = selectors::objects(store)
        .into_iter()
        .map(|o| entry(o.label(), IconName::File, PaletteChoice::Object(o.id.clone())))
        .collect();
    groups.push(Group { label: tr("palette.contacts"), entries: contacts });
    groups.push(Group { label: tr("palette.objects"), entries: objects });
    groups.push(history(store));
    groups.retain(|g| !g.entries.is_empty());
    groups
}

fn contexts(store: &Store) -> Group {
    let entries = store
        .contexts()
        .map(|c| {
            let icon = match c.kind {
                ContextKind::Direct => IconName::User,
                ContextKind::Group => IconName::Building2,
                ContextKind::Space => IconName::LayoutDashboard,
            };
            let mut e = entry(
                selectors::context_title(store, c),
                icon,
                PaletteChoice::Context(c.id.clone()),
            );
            e.keywords = c
                .participants
                .iter()
                .filter_map(|id| store.participant(id).map(|p| p.display.clone()))
                .collect();
            e
        })
        .collect();
    Group { label: tr("palette.contexts"), entries }
}

/// Most history rows the palette searches: the most recently loaded
/// message bodies (only bodies in memory are searchable).
pub const HISTORY_CAP: usize = 500;

fn history(store: &Store) -> Group {
    let mut moments: Vec<_> = store
        .bodies()
        .recent()
        .filter(|(_, text)| !text.is_empty())
        .filter_map(|(id, text)| {
            let m = store.moment(id)?;
            matches!(&m.kind, MomentKind::Message(p) if !p.tombstone).then(|| (m, text.to_owned()))
        })
        .take(HISTORY_CAP)
        .collect();
    moments.sort_by(|a, b| (b.0.at, &b.0.id).cmp(&(a.0.at, &a.0.id)));
    let entries = moments
        .into_iter()
        .map(|(m, text)| {
            let snippet: String = text.chars().take(SNIPPET).collect();
            let title = store.context(&m.context_id).map(|c| c.title.clone()).unwrap_or_default();
            let mut e = entry(
                tr_args("palette.history_row", &[("text", &snippet), ("context", &title)]),
                IconName::Inbox,
                PaletteChoice::Moment(m.context_id.clone(), m.id.clone()),
            );
            e.keywords = vec![text];
            e
        })
        .collect();
    Group { label: tr("palette.history"), entries }
}

fn actions(
    store: &Store,
    current: Option<&ContextId>,
    accounts: &[(AccountId, String)],
    active: Option<&AccountId>,
) -> Group {
    let mut entries = vec![entry(tr("palette.needs_me"), IconName::Bell, PaletteChoice::NeedsMe)];
    if let Some(context) = current.and_then(|id| store.context(id)) {
        let id = context.id.clone();
        for agent in crate::workspace::composer::assignee::agents(store, &id) {
            let mut e = entry(
                tr_args("palette.ask_agent", &[("name", &agent.name)]),
                IconName::Bot,
                PaletteChoice::AskAgent(agent.id),
            );
            e.keywords = vec![format!("@{}", agent.name)];
            entries.push(e);
        }
        entries.push(entry(
            tr_args(
                if context.pinned { "palette.unpin" } else { "palette.pin" },
                &[("title", &context.title)],
            ),
            if context.pinned { IconName::StarOff } else { IconName::Star },
            PaletteChoice::Pin(id.clone(), !context.pinned),
        ));
        entries.push(entry(
            tr_args(
                if context.muted { "palette.unmute" } else { "palette.mute" },
                &[("title", &context.title)],
            ),
            IconName::Bell,
            PaletteChoice::Mute(id.clone(), !context.muted),
        ));
        for (seconds, _) in TIMERS {
            if context.disappearing_secs == seconds {
                continue;
            }
            entries.push(entry(
                tr_args("palette.disappearing", &[("timer", &timer_label(seconds))]),
                IconName::Undo2,
                PaletteChoice::Disappearing(id.clone(), seconds),
            ));
        }
    }
    entries.push(entry(tr("palette.add_contact"), IconName::Plus, PaletteChoice::AddContact));
    entries.push(entry(tr("palette.new_group"), IconName::Plus, PaletteChoice::NewGroup));
    for tool in DockTool::ALL {
        entries.push(entry(
            tr_args("palette.open_dock", &[("dock", &tool.title())]),
            tool.icon(),
            PaletteChoice::OpenDock(tool),
        ));
    }
    for (account, label) in accounts.iter().filter(|(id, _)| Some(id) != active) {
        entries.push(entry(
            tr_args("palette.switch_account", &[("account", label)]),
            IconName::CircleUser,
            PaletteChoice::SwitchAccount(account.clone()),
        ));
    }
    entries.push(entry(tr("palette.settings"), IconName::Settings, PaletteChoice::OpenSettings));
    entries.push(entry(tr("palette.theme"), IconName::Palette, PaletteChoice::ToggleTheme));
    Group { label: tr("palette.actions"), entries }
}
