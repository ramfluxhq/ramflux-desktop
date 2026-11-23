// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Kit components in isolation, for comparison with the component boards
//! (00B, 00D, B19b). Each component renders alone in a window of its design
//! size on the board's background, with the board's label text.
//!
//! Where the application exposes the component (`style::primary_button`,
//! the catalog's `avatar`, `badge`, `trust_banner`, `page_header_content`,
//! `message_bubble`, `state_block`, the navigation `Row`, Kit checkboxes and
//! switches), the capture calls that code. Where the application builds it
//! inline in a crate-private view, the capture repeats that inline styling
//! and says from where (`replica of <file>`), so W1/W2 can measure it; the
//! replica must follow the source when the source changes. Components the
//! application does not have are listed with the reason.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyElement, App, Bounds, Entity, HeadlessAppContext, Hsla, Point, Rgba, SharedString, Window,
    WindowBounds, WindowOptions, div, px, rems, size,
};
use ramflux_desktop_navigation::generated::catalog as kit;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_ui::AppAssets as Assets;

use crate::text::t;

/// One component of the kit boards.
pub struct Component {
    pub slug: &'static str,
    /// How the capture renders it, or why it cannot.
    pub how: How,
}

pub enum How {
    /// Calls application code.
    App(&'static str, Render),
    /// Repeats inline styling of a crate-private view.
    Replica(&'static str, Render),
    /// Not capturable.
    Skip(&'static str),
}

type Render = fn(&mut Window, &mut App, &Inputs) -> AnyElement;

/// Input states some components need (created with the window).
pub struct Inputs {
    search: Entity<InputState>,
}

fn rem(design_px: f32) -> gpui_kit::Rems {
    rems(design_px / 15.0)
}

fn board_background(board: &str) -> Hsla {
    let hex: u32 = if board == "00B" { 0x00FF_FFFF } else { 0x00FC_FBF9 };
    Hsla::from(Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: 1.0,
    })
}

fn row_replica(
    title: SharedString,
    detail: SharedString,
    trailing: AnyElement,
    cx: &App,
) -> AnyElement {
    // replica of crates/ramflux-desktop-ui/src/settings/design.rs `row`
    let p = palette(cx);
    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap_4()
        .py_3()
        .border_b_1()
        .border_color(p.line)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_size(rem(p.size_body)).font_medium().child(title))
                .child(
                    div().text_size(rem(p.size_secondary)).text_color(p.secondary).child(detail),
                ),
        )
        .child(trailing)
        .into_any_element()
}

fn chip_replica(text: SharedString, cx: &App) -> AnyElement {
    // replica of crates/ramflux-desktop-ui/src/settings/design.rs `chip`
    let p = palette(cx);
    div()
        .px_2()
        .py(rem(4.0))
        .rounded(rem(p.radius_chip))
        .bg(p.surface)
        .text_size(rem(p.size_meta))
        .child(text)
        .into_any_element()
}

fn quiet_danger(label: SharedString, cx: &App) -> AnyElement {
    // replica of crates/ramflux-desktop-ui/src/settings/devices.rs revoke button
    let p = palette(cx);
    Button::new("revoke")
        .ghost()
        .h(rem(32.0))
        .px(rem(12.0))
        .bg(p.danger_bg)
        .text_color(p.danger)
        .label(label)
        .into_any_element()
}

fn bubble(
    kind: kit::BubbleKind,
    sender: Option<SharedString>,
    text: SharedString,
    time: &'static str,
    cx: &App,
) -> AnyElement {
    // content: replica of crates/ramflux-desktop-ui/src/moments/message_bubble.rs
    let p = palette(cx);
    let own = matches!(kind, kit::BubbleKind::Own);
    let meta = if own { p.outgoing_meta } else { p.incoming_meta };
    let content = div()
        .flex()
        .flex_col()
        .gap_1()
        .when_some(sender, |c, s| {
            c.child(div().text_size(rem(p.size_secondary)).font_semibold().child(s))
        })
        .child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap_2()
                .child(div().text_size(rem(p.size_body)).child(text))
                .child(div().text_size(rem(p.size_caption)).text_color(meta).child(time))
                .when(own, |r| r.child(Icon::new(IconName::CheckCheck).xsmall().text_color(meta))),
        );
    kit::message_bubble(kit::MessageContent::new("m".into(), kind, content), p).into_any_element()
}

fn tip(kind: kit::StateKind, title: SharedString, cx: &App) -> AnyElement {
    // replica of crates/ramflux-desktop-ui/src/settings/presentation.rs `notice`
    let p = palette(cx);
    div()
        .p_3()
        .rounded(rem(p.radius_md))
        .bg(p.surface)
        .child(kit::state_block("tip".into(), title, SharedString::default(), kind, div(), p))
        .into_any_element()
}

/// Every component of the kit boards, in board order.
pub fn components() -> Vec<Component> {
    use How::{App as A, Replica as R, Skip as S};
    vec![
        Component {
            slug: "button-primary",
            how: A("style::primary_button", |_, cx, _| {
                ramflux_desktop_ui::style::primary_button("b", cx)
                    .label(t("kit09"))
                    .into_any_element()
            }),
        },
        Component {
            slug: "status-neutral",
            how: A("catalog::badge", |_, cx, _| {
                kit::badge(t("kit01"), palette(cx)).into_any_element()
            }),
        },
        Component {
            slug: "avatar-identity",
            how: A("catalog::avatar Person", |_, cx, _| {
                kit::avatar("A".into(), kit::AvatarKind::Person, palette(cx)).into_any_element()
            }),
        },
        Component {
            slug: "avatar-agent",
            how: A("catalog::avatar Agent", |_, cx, _| {
                kit::avatar("N".into(), kit::AvatarKind::Agent, palette(cx)).into_any_element()
            }),
        },
        Component {
            slug: "list-row-conversation",
            how: R("workspace/context_stack.rs list row (Button + navigation Row)", |_, cx, _| {
                let p = palette(cx);
                let mut row = ramflux_desktop_navigation::row::Row::new(t("kit02"), t("kit03"));
                row.time = "09:44".into();
                Button::new("row")
                    .ghost()
                    .w_full()
                    .h(rems(ramflux_desktop_navigation::theme::ROW_REM))
                    .px_3()
                    .rounded(px(p.radius_sm))
                    .child(row.content(cx))
                    .into_any_element()
            }),
        },
        Component {
            slug: "composer-default",
            how: S(
                "the composer is a store-bound entity (workspace/composer/mod.rs); measured on the screen boards",
            ),
        },
        Component {
            slug: "menu-item-hover",
            how: S(
                "menus are GPUI Kit PopupMenu entities opened by a click; no RF menu item exists",
            ),
        },
        Component {
            slug: "menu",
            how: S("menus are GPUI Kit PopupMenu entities opened by a click; no RF menu exists"),
        },
        Component {
            slug: "info-popover",
            how: S("no info popover in the application (tooltips only)"),
        },
        Component {
            slug: "bubble-peer",
            how: R("catalog::message_bubble + moments/message_bubble.rs content", |_, cx, _| {
                bubble(kit::BubbleKind::Peer, None, t("kit10"), "14:32", cx)
            }),
        },
        Component {
            slug: "bubble-own",
            how: R("catalog::message_bubble + moments/message_bubble.rs content", |_, cx, _| {
                bubble(kit::BubbleKind::Own, None, t("kit11"), "14:33", cx)
            }),
        },
        Component {
            slug: "bubble-group",
            how: R(
                "catalog::message_bubble + moments/message_bubble.rs content (avatar not included)",
                |_, cx, _| {
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(kit::avatar(t("kit04"), kit::AvatarKind::Person, palette(cx)))
                        .child(bubble(
                            kit::BubbleKind::Peer,
                            Some(t("kit12")),
                            t("kit10"),
                            "14:32",
                            cx,
                        ))
                        .into_any_element()
                },
            ),
        },
        Component {
            slug: "navigation-item",
            how: R("workspace/navigation.rs rail item", |_, cx, _| {
                let p = palette(cx);
                div()
                    .w(rem(68.0))
                    .px_2()
                    .py(rem(20.0))
                    .bg(ramflux_desktop_navigation::theme::colors(cx).rail)
                    .child(
                        Button::new("nav")
                            .ghost()
                            .small()
                            .p_0()
                            .w_full()
                            .h(rem(52.0))
                            .rounded(px(p.radius_sm))
                            .selected(true)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap_1()
                                    .child(Icon::new(IconName::MessageSquare).with_size(px(20.)))
                                    .child(div().text_size(rem(p.size_caption)).child(t("kit13"))),
                            ),
                    )
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-secondary",
            how: A("style::secondary_button", |_, cx, _| {
                ramflux_desktop_ui::style::secondary_button("b", cx)
                    .label(t("kit14"))
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-disabled",
            how: A("style::primary_button disabled", |_, cx, _| {
                ramflux_desktop_ui::style::primary_button("b", cx)
                    .label(t("kit15"))
                    .disabled(true)
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-danger",
            how: R("Button::danger as used in call.rs, contact_profile.rs", |_, _, _| {
                Button::new("b").danger().label(t("kit16")).into_any_element()
            }),
        },
        Component {
            slug: "button-danger-quiet",
            how: R("settings/devices.rs revoke button", |_, cx, _| quiet_danger(t("kit17"), cx)),
        },
        Component { slug: "button-text", how: S("no text button in the application") },
        Component {
            slug: "icon-button",
            how: R("conversation/header.rs ellipsis button", |_, _, _| {
                Button::new("i").ghost().small().icon(IconName::Ellipsis).into_any_element()
            }),
        },
        Component {
            slug: "info-button",
            how: R("settings/nodes.rs info icon", |_, cx, _| {
                div()
                    .size(rem(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette(cx).secondary)
                    .child(Icon::new(IconName::Info).small())
                    .into_any_element()
            }),
        },
        Component {
            slug: "checkbox-on",
            how: A("Kit Checkbox", |_, _, _| Checkbox::new("c").checked(true).into_any_element()),
        },
        Component {
            slug: "checkbox-off",
            how: A("Kit Checkbox", |_, _, _| Checkbox::new("c").checked(false).into_any_element()),
        },
        Component {
            slug: "checkbox-disabled",
            how: A("Kit Checkbox", |_, _, _| {
                Checkbox::new("c").checked(false).disabled(true).into_any_element()
            }),
        },
        Component {
            slug: "radio-on",
            how: R("settings/appearance.rs radio", |_, cx, _| {
                let p = palette(cx);
                div()
                    .size(rem(18.0))
                    .rounded(rem(p.radius_full))
                    .border_1()
                    .border_color(p.primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(rem(8.0)).rounded_full().bg(p.primary))
                    .into_any_element()
            }),
        },
        Component {
            slug: "radio-off",
            how: R("settings/appearance.rs radio", |_, cx, _| {
                let p = palette(cx);
                div()
                    .size(rem(18.0))
                    .rounded(rem(p.radius_full))
                    .border_1()
                    .border_color(p.primary)
                    .into_any_element()
            }),
        },
        Component {
            slug: "switch-on",
            how: A("Kit Switch", |_, _, _| Switch::new("s").checked(true).into_any_element()),
        },
        Component {
            slug: "switch-off",
            how: A("Kit Switch", |_, _, _| Switch::new("s").checked(false).into_any_element()),
        },
        Component {
            slug: "segmented",
            how: R("settings/appearance.rs mode segments", |_, cx, _| {
                let p = palette(cx);
                let seg = |id: &'static str, label: SharedString, on: bool| {
                    Button::new(id)
                        .ghost()
                        .small()
                        .flex_1()
                        .h(rem(28.0))
                        .rounded(px(p.radius_xs))
                        .when(on, |b| b.bg(p.surface_white).font_semibold())
                        .selected(on)
                        .label(label)
                };
                div()
                    .flex()
                    .flex_row()
                    .w(rem(240.0))
                    .p(px(2.))
                    .gap(px(2.))
                    .rounded(rem(p.radius_sm))
                    .bg(p.surface)
                    .child(seg("a", t("kit18"), true))
                    .child(seg("b", t("kit19"), false))
                    .into_any_element()
            }),
        },
        Component {
            slug: "text-field",
            how: S(
                "the application's labelled field is crate-private (settings/presentation.rs `field`) and needs its form state",
            ),
        },
        Component {
            slug: "search-field",
            how: R("workspace/sections/directory.rs `search_input`", |_, cx, inputs| {
                let p = palette(cx);
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .h(rem(34.0))
                    .px_2()
                    .gap_2()
                    .rounded(rem(p.radius_input))
                    .border_1()
                    .border_color(p.line_strong)
                    .bg(p.surface_white)
                    .child(Icon::new(IconName::Search).small().text_color(p.secondary))
                    .child(
                        Input::new(&inputs.search)
                            .small()
                            .appearance(false)
                            .bordered(false)
                            .text_size(rem(p.size_secondary)),
                    )
                    .into_any_element()
            }),
        },
        Component {
            slug: "search-field-labeled",
            how: S("no labelled search field in the application"),
        },
        Component {
            slug: "dialog",
            how: S("the application has no modal dialog (inline confirm strip)"),
        },
        Component {
            slug: "dialog-destructive",
            how: S("the application has no modal dialog (inline confirm strip)"),
        },
        Component {
            slug: "security-warning-default",
            how: A("catalog::trust_banner", |_, cx, _| {
                kit::trust_banner(
                    kit::NoticeContent::new(
                        t("kit05"),
                        t("kit06"),
                        Icon::new(IconName::TriangleAlert),
                        div(),
                    ),
                    false,
                    palette(cx),
                )
                .into_any_element()
            }),
        },
        Component {
            slug: "security-warning-danger",
            how: A("catalog::trust_banner danger", |_, cx, _| {
                kit::trust_banner(
                    kit::NoticeContent::new(
                        t("kit05"),
                        t("kit06"),
                        Icon::new(IconName::ShieldAlert),
                        div(),
                    ),
                    true,
                    palette(cx),
                )
                .into_any_element()
            }),
        },
        Component {
            slug: "tip-bar-default",
            how: R("settings/presentation.rs notice (state_block Offline)", |_, cx, _| {
                tip(kit::StateKind::Offline, t("kit20"), cx)
            }),
        },
        Component {
            slug: "tip-bar-success",
            how: S("no success notice in the application (StateKind has no success)"),
        },
        Component {
            slug: "tip-bar-warning",
            how: R("settings/presentation.rs notice (state_block Loading)", |_, cx, _| {
                tip(kit::StateKind::Loading, t("kit21"), cx)
            }),
        },
        Component {
            slug: "tip-bar-danger",
            how: R("settings/presentation.rs notice (state_block Error)", |_, cx, _| {
                tip(kit::StateKind::Error, t("kit22"), cx)
            }),
        },
        Component {
            slug: "origin-display",
            how: S("the origin display has no screen (web device approval is gated)"),
        },
        Component {
            slug: "device-row",
            how: R("settings/devices.rs row via settings/design.rs `row`", |_, cx, _| {
                let trailing = div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(chip_replica(t("kit23"), cx))
                    .child(quiet_danger(t("kit17"), cx))
                    .into_any_element();
                row_replica("Chrome · macOS".into(), t("kit24"), trailing, cx)
            }),
        },
        Component {
            slug: "approval-card",
            how: S(
                "the approval card is the crate-private InterventionBody; measured on board D40",
            ),
        },
        Component { slug: "comment-row", how: S("post channels are later scope (P boards)") },
        Component { slug: "post-card", how: S("post channels are later scope (P boards)") },
        Component {
            slug: "menu-item-compact",
            how: S("no compact RF menu item in the application"),
        },
        Component {
            slug: "setting-row",
            how: R("settings/design.rs `row`", |_, cx, _| {
                row_replica(
                    t("kit25"),
                    t("kit26"),
                    ramflux_desktop_ui::style::secondary_button("c", cx)
                        .label(t("kit27"))
                        .into_any_element(),
                    cx,
                )
            }),
        },
        Component {
            slug: "step-item",
            how: S(
                "the task step row is crate-private (moments/task_progress.rs) and needs a task",
            ),
        },
        Component {
            slug: "brand-mark",
            how: S("no 28 px brand mark in the application (rail monogram and signature only)"),
        },
        Component {
            slug: "avatar-group",
            how: A("catalog::avatar Group", |_, cx, _| {
                kit::avatar("".into(), kit::AvatarKind::Group, palette(cx)).into_any_element()
            }),
        },
        Component { slug: "stage", how: S("video is later scope") },
        Component {
            slug: "chat-header",
            how: S(
                "the conversation header is crate-private and store-bound; measured on board 02",
            ),
        },
        Component { slug: "chat-header-channel", how: S("channels are later scope") },
        Component {
            slug: "page-header",
            how: A("catalog::page_header_content", |_, cx, _| {
                let p = palette(cx);
                kit::page_header_content(
                    kit::RowContent::new("ph".into(), t("kit07"), t("kit08")).trailing(
                        ramflux_desktop_ui::style::secondary_button("a", cx).label(t("kit28")),
                    ),
                    false,
                    p,
                )
                .into_any_element()
            }),
        },
        Component {
            slug: "date-separator",
            how: R("moments/mod.rs `day_header`", |_, cx, _| {
                let p = palette(cx);
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .justify_center()
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(rem(p.radius_chip))
                            .bg(p.surface)
                            .text_size(rem(p.size_meta))
                            .font_medium()
                            .text_color(p.secondary)
                            .child(t("kit29")),
                    )
                    .into_any_element()
            }),
        },
        Component { slug: "unread-divider", how: S("no unread divider in the conversation view") },
        Component {
            slug: "jump-to-latest",
            how: S("drawn by the GPUI Kit virtual list, shown only when scrolled"),
        },
    ]
}

struct KitView {
    background: Hsla,
    render: Render,
    inputs: Inputs,
}

impl gpui_kit::Render for KitView {
    fn render(
        &mut self,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        let element = (self.render)(window, cx, &self.inputs);
        div().size_full().bg(self.background).flex().flex_col().items_start().child(element)
    }
}

/// Renders `render` alone in a `w` x `h` window on `board`'s background.
pub fn capture(board: &str, w: u32, h: u32, render: Render) -> Result<image::RgbaImage, String> {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    let fonts = cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        ramflux_desktop_ui::i18n::select_locale("zh-CN", cx);
        ramflux_desktop_ui::assets::load_fonts(cx)
    });
    fonts.map_err(|e| format!("bundled fonts: {e}"))?;
    let background = board_background(board);
    let opened = cx.update(|cx| {
        let bounds = Bounds { origin: Point::default(), size: size(px(w as f32), px(h as f32)) };
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: false,
                show: false,
                ..Default::default()
            },
            cx,
            move |window, cx| {
                let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("kit30")));
                cx.new(|_| KitView { background, render, inputs: Inputs { search } })
            },
        )
    });
    let (handle, _view) = opened.map_err(|e| format!("offscreen window: {e}"))?;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .map_err(|e| format!("window: {e}"))?;
    cx.capture_screenshot(handle).map_err(|e| format!("render: {e}"))
}
