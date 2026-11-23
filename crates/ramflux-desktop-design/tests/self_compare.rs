// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The comparison on a real capture of the shell: a capture compared with
//! itself has no difference, and the same capture moved by a few pixels is
//! detected. macOS only (the headless Metal renderer); elsewhere the
//! synthetic tests in `src/compare.rs` cover the same rules. The macOS
//! platform must be created on the main thread, so this test has its own
//! `main` (`harness = false`).

fn main() {
    #[cfg(target_os = "macos")]
    a_real_capture_matches_itself_and_a_shift_is_detected();
    #[cfg(not(target_os = "macos"))]
    println!("self_compare: skipped; the headless capture test runs on macOS");
}

#[cfg(target_os = "macos")]
fn a_real_capture_matches_itself_and_a_shift_is_detected() {
    use std::sync::Arc;

    use gpui_kit::{
        AppContext as _, Bounds, HeadlessAppContext, Point, WindowBounds, WindowOptions, px, size,
    };
    use ramflux_desktop_design::compare::{Options, compare, flatten};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;

    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        ramflux_desktop_ui::assets::load_fonts(cx).unwrap();
    });
    let (handle, _shell) = cx
        .update(|cx| {
            let bounds = Bounds { origin: Point::default(), size: size(px(800.), px(600.)) };
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Shell::with_scenario("s02_daily", false, window, cx)),
            )
        })
        .unwrap();
    cx.run_until_parked();
    let image = flatten(&cx.capture_screenshot(handle).unwrap());
    let scale = image.width() / 800;
    assert!(scale >= 1);

    let same = compare(&image, &image.clone(), scale, Options::default());
    assert_eq!(same.differing, 0);
    assert!(same.regions.is_empty());
    assert!((same.ssim - 1.0).abs() < 1e-9);

    // The whole screen moved 3 design pixels right.
    let shift = 3 * scale;
    let mut moved = image.clone();
    for y in 0..image.height() {
        for x in 0..image.width() {
            let from = x.saturating_sub(shift);
            moved.put_pixel(x, y, *image.get_pixel(from, y));
        }
    }
    let shifted = compare(&image, &moved, scale, Options::default());
    assert!(shifted.differing > 0, "a 3 px shift must be detected");
    assert!(!shifted.regions.is_empty());
    assert!(shifted.ssim < 0.99);
    println!(
        "self_compare: identical capture 0 differing px; 3 px shift {} differing px in {} regions, ssim {:.3}",
        shifted.differing,
        shifted.regions.len(),
        shifted.ssim
    );
}
