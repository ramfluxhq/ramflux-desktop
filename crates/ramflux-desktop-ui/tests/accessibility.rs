// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Accessibility state in painted frames: every interactive control on every
//! page has an AccessKit role and a non-empty name, moments announce their
//! kind and state, the hold strip announces its request and risk, and
//! list placeholders are status nodes.

mod common;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Entity, Role, TestAppContext, WindowHandle};
use ramflux_desktop_core::{Lifecycle, Risk};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::{self, tr};
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::DockTool;

type Handle = WindowHandle<gpui_kit::base::Root>;

/// Roles a person operates; each must carry a name.
const INTERACTIVE: [Role; 9] = [
    Role::Button,
    Role::TextInput,
    Role::PasswordInput,
    Role::CheckBox,
    Role::Switch,
    Role::ComboBox,
    Role::Tab,
    Role::TreeItem,
    Role::Link,
];

/// The dock engine's own tab strip (tabs and the overflow menu of a tab
/// group) is drawn inside GPUI Kit's `DockArea`, which offers no way to name
/// those controls (upstream gap). Every tool they reach is also reachable by
/// the named rail buttons and by secondary-alt-1..5, so they are left out of
/// this audit and reported instead.
fn dock_engine_chrome(path: &[gpui_kit::ElementId]) -> bool {
    let in_tab_group = path.iter().any(|id| id.to_string() == "tab-panel");
    let last = path.last().map(ToString::to_string).unwrap_or_default();
    in_tab_group && (last == "menu" || last.parse::<usize>().is_ok())
}

/// Every visible interactive control without a name, as `role path`.
fn unnamed(cx: &mut TestAppContext, handle: Handle) -> Vec<String> {
    in_window(cx, handle, |window, _| {
        INTERACTIVE
            .iter()
            .flat_map(|role| window.find_all_by_role(*role))
            .filter(|e| e.visible() && !dock_engine_chrome(e.path()))
            .filter(|e| e.label().is_none_or(|l| l.trim().is_empty()))
            .map(|e| format!("{:?} {:?}", e.role(), e.path().last()))
            .collect()
    })
}

fn named_controls(cx: &mut TestAppContext, handle: Handle) -> usize {
    in_window(cx, handle, |window, _| {
        INTERACTIVE.iter().map(|role| window.find_all_by_role(*role).len()).sum()
    })
}

fn audit(cx: &mut TestAppContext, handle: Handle, page: &str) {
    let missing = unnamed(cx, handle);
    assert!(missing.is_empty(), "{page}: controls without a name: {missing:#?}");
    assert!(named_controls(cx, handle) > 0, "{page}: no controls found");
}

fn open_tool(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>, tool: DockTool) {
    let workspace = workspace(cx, shell);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_tool(tool, window, cx));
    })
    .expect("window");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn every_control_on_the_entry_steps_has_a_role_and_a_name(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s01_first_launch");
    advance(cx, 100);
    audit(cx, handle, "entry daemon step");
    advance(cx, 2_900);
    audit(cx, handle, "entry node step");
    click_in(cx, handle, "node-n-harbor", "node-confirm");
    advance(cx, 300);
    audit(cx, handle, "entry account step");
    let _ = shell;
}

#[gpui_kit::test]
fn every_control_in_the_workspace_docks_palette_and_settings_is_named(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    audit(cx, handle, "workspace");
    for tool in DockTool::ALL {
        open_tool(cx, handle, &shell, tool);
        audit(cx, handle, &format!("dock {tool:?}"));
    }
    in_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ramflux_desktop_ui::actions::OpenPalette), cx);
    });
    audit(cx, handle, "palette");
    in_window(cx, handle, |window, cx| {
        window.dispatch_action(Box::new(ramflux_desktop_ui::actions::ClosePalette), cx);
    });
    for section in SettingsSection::ALL {
        cx.update(|cx| shell.update(cx, |shell, cx| shell.open_settings_at(section, cx)));
        cx.run_until_parked();
        audit(cx, handle, &format!("settings {section:?}"));
    }
}

#[gpui_kit::test]
fn moments_announce_kind_state_and_risk_and_the_strip_its_risk(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let (role, label) = in_window(cx, handle, |window, _| {
        let row = window.find("moment-m-approve");
        (row.role(), row.label().map(str::to_owned).unwrap_or_default())
    });
    assert_eq!(role, Some(Role::ListItem));
    let kind = i18n::moment_kind(&ramflux_desktop_core::MomentKind::Intervention(
        ramflux_desktop_core::InterventionPayload {
            grant_id: "g".into(),
            risk: Risk::High,
            summary: String::new(),
        },
    ));
    for part in [kind.to_string(), i18n::lifecycle(Lifecycle::NeedsMe).to_string()] {
        assert!(label.contains(&part), "the row names {part:?}: {label}");
    }
    let risk =
        with_store(cx, &shell, |s| s.grant(&"g-rotate".into()).map(|g| g.risk).expect("grant"));
    assert!(label.contains(i18n::risk(risk).as_ref()), "and its risk: {label}");

    click_in(cx, handle, "moment-m-approve", "approve");
    let (role, label) = in_window(cx, handle, |window, _| {
        let strip = window.find("confirm-strip");
        (strip.role(), strip.label().map(str::to_owned).unwrap_or_default())
    });
    assert_eq!(role, Some(Role::Alert), "the strip is announced");
    assert!(label.contains(i18n::risk(risk).as_ref()), "with its risk: {label}");
    let hold = in_window(cx, handle, |window, _| {
        let hold = window.find("hold-confirm");
        (hold.role(), hold.label().map(str::to_owned))
    });
    assert_eq!(hold, (Some(Role::Button), Some(tr("confirm.hold").to_string())));
}

#[gpui_kit::test]
fn list_placeholders_are_status_nodes(cx: &mut TestAppContext) {
    let (handle, _shell) = open(cx, "s01_first_launch");
    advance(cx, 1_600);
    let role = in_window(cx, handle, |window, _| window.find("entry-node-loading").role());
    assert_eq!(role, Some(Role::Status));
}
