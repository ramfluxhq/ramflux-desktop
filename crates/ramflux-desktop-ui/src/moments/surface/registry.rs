// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The block renderer registry: one entry per block kind this client draws,
//! with the data versions it reads.
//!
//! It is the single source of truth for the renderer profile: [`profile`]
//! is built from [`REGISTRY`], and [`render`] draws only through it, so the
//! profile can never claim a kind the renderer cannot draw. A kind or
//! version missing here is unsupported: its block renders as a fallback and
//! is recorded as a renderer error.

use std::sync::LazyLock;

use gpui_kit::{AnyElement, App};
use ramflux_desktop_core::conformance::{
    BlockSupport, DeviceClass, InputMode, Limits, OutputMode, SurfaceMode,
};
use ramflux_desktop_core::{Block, BlockKind, RendererProfile};

use super::{BlockContext, content, feedback, interactive};

/// Draws one block, or `None` when its data cannot be read.
pub type RenderFn = fn(&Block, &BlockContext<'_>, &App) -> Option<AnyElement>;

/// One registered block renderer.
pub struct Renderer {
    /// Kind.
    pub kind: BlockKind,
    /// Data versions it reads.
    pub versions: &'static [u32],
    /// The renderer.
    pub render: RenderFn,
}

const V1: &[u32] = &[1];

/// Every block renderer of this client, in catalog order.
pub const REGISTRY: &[Renderer] = &[
    Renderer { kind: BlockKind::Text, versions: V1, render: |b, _, cx| content::text(&b.data, cx) },
    Renderer {
        kind: BlockKind::Markdown,
        versions: V1,
        render: |b, _, cx| content::markdown(&b.id, &b.data, cx),
    },
    Renderer {
        kind: BlockKind::Heading,
        versions: V1,
        render: |b, _, cx| content::heading(&b.data, cx),
    },
    Renderer {
        kind: BlockKind::KeyValue,
        versions: V1,
        render: |b, _, _| content::key_value(&b.data),
    },
    Renderer {
        kind: BlockKind::Metric,
        versions: V1,
        render: |b, _, cx| content::metric(&b.data, cx),
    },
    Renderer { kind: BlockKind::List, versions: V1, render: |b, _, cx| content::list(&b.data, cx) },
    Renderer {
        kind: BlockKind::TableFallback,
        versions: V1,
        render: |b, _, cx| content::table(&b.data, cx),
    },
    Renderer {
        kind: BlockKind::Form,
        versions: V1,
        render: |b, _, cx| interactive::form(&b.data, cx),
    },
    Renderer {
        kind: BlockKind::Input,
        versions: V1,
        render: |b, context, cx| interactive::input(&b.data, context, cx),
    },
    Renderer {
        kind: BlockKind::Select,
        versions: V1,
        render: |b, context, cx| interactive::select(&b.data, context, cx),
    },
    Renderer {
        kind: BlockKind::Progress,
        versions: V1,
        render: |b, _, cx| feedback::progress(&b.id, &b.data, cx),
    },
    Renderer {
        kind: BlockKind::LogStream,
        versions: V1,
        render: |b, _, cx| feedback::log_stream(&b.data, cx),
    },
    Renderer {
        kind: BlockKind::Warning,
        versions: V1,
        render: |b, _, _| feedback::callout(&b.id, b.kind, &b.data),
    },
    Renderer {
        kind: BlockKind::Error,
        versions: V1,
        render: |b, _, _| feedback::callout(&b.id, b.kind, &b.data),
    },
    Renderer {
        kind: BlockKind::Success,
        versions: V1,
        render: |b, _, _| feedback::callout(&b.id, b.kind, &b.data),
    },
    Renderer {
        kind: BlockKind::Approval,
        versions: V1,
        render: |b, context, cx| interactive::approval(&b.data, context, cx),
    },
    Renderer {
        kind: BlockKind::ActionGroup,
        versions: V1,
        render: |b, context, cx| interactive::action_group(&super::action_ids(b), context, cx),
    },
    Renderer {
        kind: BlockKind::FilePreview,
        versions: V1,
        render: |b, _, cx| content::file_preview(&b.data, cx),
    },
    Renderer {
        kind: BlockKind::Fallback,
        versions: V1,
        render: |_, context, cx| Some(content::fallback(context.fallback, cx)),
    },
];

/// The registered renderer for `kind` at data `version`.
pub fn renderer(kind: BlockKind, version: u32) -> Option<&'static Renderer> {
    REGISTRY.iter().find(|r| r.kind == kind && r.versions.contains(&version))
}

/// Draws `block` through the registry; `None` when no renderer reads it or
/// its data cannot be read (the caller shows the fallback block).
pub fn render(block: &Block, context: &BlockContext<'_>, cx: &App) -> Option<AnyElement> {
    renderer(block.kind, block.version).and_then(|r| (r.render)(block, context, cx))
}

static PROFILE: LazyLock<RendererProfile> = LazyLock::new(|| RendererProfile {
    device_class: DeviceClass::Desktop,
    runtime: "gpui_kit_native".to_owned(),
    blocks: REGISTRY
        .iter()
        .map(|r| BlockSupport { kind: r.kind, versions: r.versions.to_vec() })
        .collect(),
    input: vec![InputMode::Pointer, InputMode::Keyboard],
    output: vec![OutputMode::Visual],
    // Surfaces render inline only; detaching into windows is not built yet.
    surface_modes: vec![SurfaceMode::Inline],
    limits: Limits::DESKTOP,
});

/// This client's renderer profile, built from [`REGISTRY`].
pub fn profile() -> &'static RendererProfile {
    &PROFILE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_claims_exactly_what_the_registry_draws() {
        let profile = profile();
        assert_eq!(profile.blocks.len(), REGISTRY.len());
        for support in &profile.blocks {
            for version in &support.versions {
                assert!(renderer(support.kind, *version).is_some(), "{:?}", support.kind);
            }
        }
        assert!(!profile.supports(BlockKind::Unsupported, 1));
        assert!(!profile.supports(BlockKind::Text, 2));
        assert_eq!(profile.limits, Limits::DESKTOP, "policy enforces the same limits");
    }
}
