// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Kit components in isolation, for comparison with the component boards
//! (00B, 00D, B19b). Each component renders alone in a window of its design
//! size on the board's background, with the board's label text.
//!
//! Where the application exposes the component (`style::primary_button`,
//! the catalog's `avatar`, `badge`, `trust_banner`, `page_header`,
//! `message_shell`, `state_block`, the navigation `Row`, design checkbox and radio, Kit
//! switches), the capture calls that code. Where the application builds it
//! inline in a crate-private view, the capture repeats that inline styling
//! and says from where (`replica of <file>`), so W1/W2 can measure it; the
//! replica must follow the source when the source changes. Components the
//! application does not have are listed with the reason.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Selectable as _, Sizable as _, StyledExt as _};
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyElement, App, Bounds, Entity, HeadlessAppContext, Hsla, Point, Rgba, SharedString, Window,
    WindowBounds, WindowOptions, div, px, rems, size,
};
use ramflux_desktop_navigation::generated::catalog as kit;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_navigation::scale::{design_px, space};
use ramflux_desktop_ui::AppAssets as Assets;
use ramflux_desktop_ui::controls::{DesignCheckbox, radio_indicator};
use ramflux_desktop_ui::message_shell::{BubbleKind, message_avatar, message_shell};
use ramflux_desktop_ui::status::{Tone, status_chip, tip_bar};
use ramflux_desktop_ui::style::DesignButton as _;

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
                .gap(space(1.))
                .child(div().text_size(rem(p.size_body)).font_medium().child(title))
                .child(
                    div().text_size(rem(p.size_secondary)).text_color(p.secondary).child(detail),
                ),
        )
        .child(trailing)
        .into_any_element()
}

fn bubble(
    kind: BubbleKind,
    sender: Option<SharedString>,
    text: SharedString,
    time: &'static str,
    width: f32,
    cx: &App,
) -> AnyElement {
    // The component boards draw a bubble in a fixed frame (360 px wide, time
    // at the right edge); on screens a bubble hugs its text up to
    // `rf-bubble-max`.
    // content: replica of crates/ramflux-desktop-ui/src/moments/message_bubble.rs
    // (`render`) and moments/message.rs (the body in the message style)
    use ramflux_desktop_navigation::scale::{DesignText as _, style};
    let p = palette(cx);
    let own = matches!(kind, BubbleKind::Own);
    let meta = if own { p.outgoing_meta } else { p.incoming_meta };
    let content = div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(space(1.))
        .when_some(sender, |c, s| {
            c.child(
                div()
                    .design_text(style::secondary().weight(gpui_kit::FontWeight::SEMIBOLD))
                    .child(s),
            )
        })
        .child(
            div()
                .flex()
                .flex_row()
                .w_full()
                .min_w_0()
                .items_end()
                .gap(space(2.))
                .child(div().flex_1().min_w_0().design_text(style::body().line(24.0)).child(text))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .relative()
                        .top(design_px(1.))
                        .gap(space(1.))
                        .design_text(style::caption().line(24.0))
                        .text_color(meta)
                        .child(time)
                        .when(own, |r| {
                            r.child(
                                Icon::new(IconName::CheckCheck)
                                    .size(design_px(14.0))
                                    .text_color(meta),
                            )
                        }),
                ),
        );
    message_shell("m", kind, content, cx).w(rem(width)).into_any_element()
}

fn tip(tone: Tone, title: SharedString, cx: &App) -> AnyElement {
    div().w(rem(480.0)).child(tip_bar(tone, None, title, cx)).into_any_element()
}

/// Every component of the kit boards, in board order.
pub fn components() -> Vec<Component> {
    use How::{App as A, Replica as R, Skip as S};
    vec![
        Component {
            slug: "button-primary",
            how: A("style::primary_button", |_, cx, _| {
                ramflux_desktop_ui::style::primary_button("b", cx)
                    .design_label(t("kit09"))
                    .into_any_element()
            }),
        },
        Component {
            slug: "status-neutral",
            how: A("status::status_chip Neutral", |_, cx, _| {
                status_chip(Tone::Neutral, t("kit01"), cx).into_any_element()
            }),
        },
        Component {
            slug: "avatar-identity",
            how: A("avatar::avatar Person", |_, cx, _| {
                ramflux_desktop_ui::avatar::avatar("A".into(), kit::AvatarKind::Person, palette(cx))
                    .into_any_element()
            }),
        },
        Component {
            slug: "avatar-agent",
            how: A("avatar::avatar Agent", |_, cx, _| {
                ramflux_desktop_ui::avatar::avatar("N".into(), kit::AvatarKind::Agent, palette(cx))
                    .into_any_element()
            }),
        },
        Component {
            slug: "list-row-conversation",
            how: R("workspace/context_stack.rs list row (Button + navigation Row)", |_, cx, _| {
                let p = palette(cx);
                // The component shows no avatar and no time.
                let mut row = ramflux_desktop_navigation::row::Row::new(t("kit02"), t("kit03"));
                row.avatar = false;
                Button::new("row")
                    .ghost()
                    .w_full()
                    .h(rems(ramflux_desktop_navigation::theme::ROW_REM))
                    .px(space(3.))
                    .rounded(px(p.radius_md))
                    .selected(true)
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
            how: R("message_shell + moments/message_bubble.rs content", |_, cx, _| {
                bubble(BubbleKind::Peer, None, t("kit10"), "14:32", 360.0, cx)
            }),
        },
        Component {
            slug: "bubble-own",
            how: R("message_shell + moments/message_bubble.rs content", |_, cx, _| {
                bubble(BubbleKind::Own, None, t("kit11"), "14:33", 360.0, cx)
            }),
        },
        Component {
            slug: "bubble-group",
            how: R(
                "message_shell, message_avatar + moments/message_bubble.rs content",
                |_, cx, _| {
                    div()
                        .flex()
                        .flex_row()
                        .gap(space(2.))
                        .child(message_avatar(t("kit04"), false, cx))
                        .child(bubble(
                            BubbleKind::Peer,
                            Some(t("kit12")),
                            t("kit10"),
                            "14:32",
                            360.0,
                            cx,
                        ))
                        .into_any_element()
                },
            ),
        },
        Component {
            slug: "navigation-item",
            how: A("workspace::rail_item", |_, cx, _| {
                // The component (00B) is 68 wide with a "68px" caption above
                // the 52 px item, on no fill.
                div()
                    .w(rem(68.0))
                    .px(rem(8.0))
                    .pt(rem(32.0))
                    .child(ramflux_desktop_ui::workspace::rail_item(
                        "nav",
                        "navigation.messages",
                        IconName::MessagesSquare,
                        true,
                        cx,
                    ))
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-secondary",
            how: A("style::secondary_button", |_, cx, _| {
                ramflux_desktop_ui::style::secondary_button("b", cx)
                    .design_label(t("kit14"))
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-disabled",
            how: A("style::primary_button design_disabled", |_, cx, _| {
                ramflux_desktop_ui::style::primary_button("b", cx)
                    .design_label(t("kit15"))
                    .design_disabled(true, cx)
                    .into_any_element()
            }),
        },
        Component {
            slug: "button-danger",
            how: A("style::DesignButton::design_danger", |_, cx, _| {
                Button::new("b").design_danger(cx).design_label(t("kit16")).into_any_element()
            }),
        },
        Component {
            slug: "button-danger-quiet",
            how: A("style::DesignButton::design_danger_quiet", |_, cx, _| {
                Button::new("b").design_danger_quiet(cx).design_label(t("kit17")).into_any_element()
            }),
        },
        Component {
            slug: "button-text",
            how: A("style::DesignButton::design_text", |_, cx, _| {
                Button::new("b").design_text(cx).design_label(t("kit31")).into_any_element()
            }),
        },
        Component {
            slug: "icon-button",
            how: R("conversation/header.rs ellipsis button", |_, cx, _| {
                Button::new("i")
                    .ghost()
                    .small()
                    .size(rem(32.0))
                    .child(
                        Icon::new(IconName::Ellipsis)
                            .size(rem(20.0))
                            .text_color(palette(cx).secondary),
                    )
                    .into_any_element()
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
                    .child(Icon::new(IconName::Info).size(rem(18.0)))
                    .into_any_element()
            }),
        },
        Component {
            slug: "checkbox-on",
            how: A("controls::DesignCheckbox", |_, _, _| {
                DesignCheckbox::new("c").checked(true).into_any_element()
            }),
        },
        Component {
            slug: "checkbox-off",
            how: A("controls::DesignCheckbox", |_, _, _| {
                DesignCheckbox::new("c").checked(false).into_any_element()
            }),
        },
        Component {
            slug: "checkbox-disabled",
            how: A("controls::DesignCheckbox", |_, _, _| {
                DesignCheckbox::new("c").checked(false).disabled(true).into_any_element()
            }),
        },
        Component {
            slug: "radio-on",
            how: A("controls::radio_indicator", |_, cx, _| radio_indicator(true, cx)),
        },
        Component {
            slug: "radio-off",
            how: A("controls::radio_indicator", |_, cx, _| radio_indicator(false, cx)),
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
                        .text_color(if on { p.text } else { p.secondary })
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
                    .px(rem(8.0))
                    .gap(rem(6.0))
                    .rounded(rem(p.radius_input))
                    .border_1()
                    .border_color(p.line_strong)
                    .bg(p.surface_white)
                    .child(Icon::new(IconName::Search).size(rem(16.0)).text_color(p.secondary))
                    .child(
                        Input::new(&inputs.search)
                            .small()
                            .appearance(false)
                            .bordered(false)
                            .px_0()
                            .text_size(rem(p.size_button)),
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
            how: A("status::security_notice", |_, cx, _| {
                div()
                    .w(rem(480.0))
                    .child(ramflux_desktop_ui::status::security_notice(
                        false,
                        t("kit05"),
                        t("kit06"),
                        cx,
                    ))
                    .into_any_element()
            }),
        },
        Component {
            slug: "security-warning-danger",
            how: A("status::security_notice", |_, cx, _| {
                div()
                    .w(rem(480.0))
                    .child(ramflux_desktop_ui::status::security_notice(
                        true,
                        t("kit05"),
                        t("kit06"),
                        cx,
                    ))
                    .into_any_element()
            }),
        },
        Component {
            slug: "tip-bar-default",
            how: A("status::tip_bar Neutral", |_, cx, _| tip(Tone::Neutral, t("kit20"), cx)),
        },
        Component {
            slug: "tip-bar-success",
            how: A("status::tip_bar Success", |_, cx, _| tip(Tone::Success, t("kit32"), cx)),
        },
        Component {
            slug: "tip-bar-warning",
            how: A("status::tip_bar Warning", |_, cx, _| tip(Tone::Warning, t("kit21"), cx)),
        },
        Component {
            slug: "tip-bar-danger",
            how: A("status::tip_bar Danger", |_, cx, _| tip(Tone::Danger, t("kit22"), cx)),
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
                    .child(status_chip(Tone::Warning, t("kit23"), cx))
                    .child(Button::new("revoke").design_danger_quiet(cx).design_label(t("kit17")))
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
            how: A("avatar::avatar Group (no initial)", |_, cx, _| {
                ramflux_desktop_ui::avatar::avatar("".into(), kit::AvatarKind::Group, palette(cx))
                    .into_any_element()
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
            how: A("page_header::page_header", |_, cx, _| {
                ramflux_desktop_ui::page_header::page_header(
                    t("kit07"),
                    ramflux_desktop_navigation::scale::style::subheading(),
                    t("kit08"),
                    None,
                    Button::new("a").design_text(cx).design_label(t("kit28")),
                    palette(cx),
                )
                .into_any_element()
            }),
        },
        Component {
            slug: "date-separator",
            how: R("moments/mod.rs `day_header`", |_, cx, _| {
                use ramflux_desktop_navigation::scale::{DesignText as _, style};
                let p = palette(cx);
                div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .justify_center()
                    .child(
                        div()
                            .px(space(2.))
                            .py(space(0.5))
                            .rounded(rem(p.radius_chip))
                            .bg(p.surface)
                            .design_text(style::meta().line(16.0))
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
    /// The component's design size; the window is larger so the pointer
    /// can rest outside the component (no hover state).
    width: f32,
    height: f32,
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
        // The shell's interface font (Hanken Grotesk with Noto Sans SC).
        div()
            .size_full()
            .font(ramflux_desktop_ui::assets::design_font(gpui_kit::FontWeight::NORMAL))
            .bg(self.background)
            .child(
                div()
                    .w(px(self.width))
                    .h(px(self.height))
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(element),
            )
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
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px((w + MARGIN) as f32), px((h + MARGIN) as f32)),
        };
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
                cx.new(|_| KitView {
                    background,
                    width: w as f32,
                    height: h as f32,
                    render,
                    inputs: Inputs { search },
                })
            },
        )
    });
    let (handle, _view) = opened.map_err(|e| format!("offscreen window: {e}"))?;
    cx.run_until_parked();
    // The pointer rests in the margin, outside the component.
    cx.update_window(handle, |_, window, cx| {
        use gpui_kit::InputEvent as _;
        window.render_frame(cx);
        let rest = gpui_kit::point(px((w + MARGIN / 2) as f32), px((h + MARGIN / 2) as f32));
        window.dispatch_event(
            gpui_kit::MouseMoveEvent { position: rest, ..Default::default() }.to_platform_input(),
            cx,
        );
    })
    .map_err(|e| format!("window: {e}"))?;
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .map_err(|e| format!("window: {e}"))?;
    let image = cx.capture_screenshot(handle).map_err(|e| format!("render: {e}"))?;
    let scale = image.width() / (w + MARGIN).max(1);
    Ok(image::imageops::crop_imm(&image, 0, 0, w * scale, h * scale).to_image())
}

/// Extra window space around a component for the resting pointer.
const MARGIN: u32 = 16;
