# Design instrument

Screens are measured against the design boards automatically. The
instrument lives in `crates/ramflux-desktop-design` and has four parts:

| Part | Where | Role |
|---|---|---|
| References | `design/boards/manifest.json` + the exported set (`RAMFLUX_DESIGN_REFS`) | Every exported board image pinned by key, node id, size, scale and SHA-256, plus the design file's SHA-256 |
| Capture | `examples/board_capture/` | Renders each mapped screen over the mock source, headless, at the board size |
| Compare | `rf-design compare` | Pixel difference, perceptual score, regions, three-up images, result and report |
| Reviewed deviations | `design/deviations/<board-key>.toml` | Known differences per board, with class, reason and reviewer decision |
| Attribution | `rf-design compare --classify` | Error class of every unreviewed region, by measurement |
| Kit components | `design/kit/components.toml`, `board_capture --kit`, `rf-design kit` | Each kit component alone against the component boards |
| Tokens | `rf-design tokens`, `design/pencil/variables.json` | The code's design tokens compared by value with the 100 design variables |

The per-board rule comes from the UI development flow: a deviation over 2 px
or a token mismatch is fixed, or listed with a reason and a decision.

## References

The PNG exports are **not** vendored. The priority set alone (86 boards at 2x
and 1x) is 26 MB, more than the whole repository history (21 MB), and the
full export is 73 MB. The repository keeps only the manifest (300 KB); the
images come from the design reference set exported from the design file
(`boards-desktop/` with its own `manifest.json`), found through an
environment variable:

```sh
export RAMFLUX_DESIGN_REFS=/path/to/reference/design/boards-desktop
cargo run -p ramflux-desktop-design --release --bin rf-design -- refs verify            # all 586 images
cargo run -p ramflux-desktop-design --release --bin rf-design -- refs verify --priority # the 86-board set
```

`refs verify` fails on a missing image, a changed SHA-256, a wrong size, or a
reference set exported from another design file. After a reviewed re-export,
`rf-design refs import --from <boards-desktop>` rewrites the manifest; commit
it together with the deviation files the new references change. References
are never edited by hand.

## Capture

```sh
TZ=UTC cargo run -p ramflux-desktop-design --example board_capture -- --out /tmp/board-captures
```

- One capture per board key (default: the priority set), named
  `<key>__<locale>__<theme>__<width>x<height>.png`, plus `captures.json`
  with the recipe note, missing landmarks and every skipped board with its
  reason.
- Headless: GPUI's headless renderer on the test platform (no window
  appears), scale factor 2, so captures are 2x and compare with the 2x
  references. The size is the board frame, also for boards drawn taller than
  a window (1040 and 1120 canvases), so bottom-anchored elements sit where
  the board draws them; those boards are compared over their top 900 px
  (`crop: top` in the result, lead decision).
- Deterministic: bundled fonts (Hanken Grotesk, Noto Sans SC), the mock's
  scenario clock or the test dispatcher's clock (fixture epoch
  2026-10-01T00:00Z), and `TZ=UTC` for day labels (the program cannot set it
  itself; it records the value). Re-running gives byte-identical images; the
  call bar counts on the session clock (scenario time on the mock). The
  exception is loading states (02L, F28, G27): their spinners animate on
  real time, so a few pixels inside the already recorded spinner region
  change from run to run; `--check` stays green.
- Board copy lives in `examples/board_capture/fixtures/board_text.toml` and
  `board_zh.ron`: Rust sources carry no CJK text (`ui/tests/locales.rs`).
- Theme and locale are applied the way the Appearance page does; the default
  is the board's own theme and language (zh-CN for almost every board, en for
  I12 and I33).
- Content comes from `examples/board_capture/fixtures/board_zh.ron`, a mock
  scenario that reproduces the boards in Simplified Chinese: the account
  林澈 with the identities 个人 and 工作, the conversation list and the
  产品设计 conversation of board 02, contacts and strangers (07, F17),
  the agents Nova, Atlas and Mira with Nova's approval cards (D40), the task
  of 06, the files of H01 and the devices of G08. It is tooling only (an
  example of this crate) and never part of the application. Recipes derive
  variants from it (no strangers for the message boards, a key change for
  B10, no devices for G27–G29); the empty-list states use an empty account,
  the entry boards the mock ceremony with the account name 个人, and A16 the
  real launcher with this program standing in as a daemon that exits at once.
- Each capture opens a fresh application and window; a recipe that fails or
  panics fails that capture only (exit status 1, reason in `captures.json`).
- Runs over a few boards add to an existing `captures.json`, so long runs can
  be split into batches.

### Capture matrix

The same program takes the matrix as parameters:

| Dimension | Values |
|---|---|
| Theme | `light`, `dark` |
| Locale | `zh-CN`, `en`, `en-XA` (long pseudo-locale), `ar-XB` (right-to-left pseudo-locale); a locale the application does not ship yet is skipped and logged |
| Size | `board`, `1280x820` (default window), `1024x768` and `800x900` (compact layouts), `640x480` (minimum window) |

```sh
TZ=UTC cargo run -p ramflux-desktop-design --example board_capture -- --out /tmp/matrix \
  --boards 02,G13 --themes light,dark --locales zh-CN,en,en-XA,ar-XB \
  --sizes board,1280x820,1024x768,800x900,640x480
```

Only the board cell (board theme, board language, board size) is compared
with a reference. The other cells are for review of clipping, overflow and
mirroring, and for regression comparisons between runs.

## Compare

```sh
cargo run -p ramflux-desktop-design --release --bin rf-design -- compare \
  --captures /tmp/board-captures --out /tmp/board-compare [--review]
```

1. A 2x capture is compared with the board's 2x reference when one exists,
   otherwise box-averaged to 1x. Images are aligned at the top-left corner.
2. A pixel differs when its largest channel difference exceeds 32/255 and the
   other image has no pixel within that tolerance in its one design pixel
   neighbourhood, in both directions. One-pixel anti-aliasing and edge
   shifts pass; larger shifts and colour changes do not.
3. Differing pixels are grouped into 4 px cells, cells into 8-connected
   regions, regions closer than 8 px are merged. A region larger than 2 design
   px in either direction is a deviation.
4. Reported per board: size match and crop, diff (share of differing pixels
   with the tolerance), strict (threshold only), tint (share beyond 4/255:
   surface colours too close for the main threshold, such as a wrong
   background token), mean SSIM of the luminance over 8x8 windows (1.000 is
   identical), the regions in design px, and a three-up image
   (reference | capture | heat map). In the heat map differing pixels run
   from yellow to red over the faded capture; outlines are amber for known
   regions, magenta for NEW ones, blue for resolved entries.

Output: `result.json` (machine-readable), `report.md` (worst first),
`three-up/<capture>.png`, and with `--review` `review.md`.

## Kit components

```sh
TZ=UTC cargo run -p ramflux-desktop-design --example board_capture -- --kit --out /tmp/kit
cargo run -p ramflux-desktop-design --release --bin rf-design -- kit --captures /tmp/kit --out /tmp/kit-compare
```

`design/kit/components.toml` lists the 56 components of the boards 00B, 00D
and B19b with their rectangles (measured on the 1x references). The capture
renders each component alone at its design size on the board's background:
through application code where the application exposes it (`app`), by
repeating the inline styling of a crate-private view with its source named
(`replica`; keep it in step with that source), or not at all with the reason
(`skipped`: no such component in the application, store-bound views,
later-scope features). `rf-design kit` crops the reference, compares at 1x
and writes `kit.md` (worst first), `kit.json` and three-up images.

## Reviewed deviations

One TOML file per board in `design/deviations/`:

```toml
board = "02"
capture = "02__zh-CN__light__1440x900"
reference_sha256 = "…"

[[deviation]]
id = "r01"
region = { x = 498, y = 110, w = 828, h = 199 }
class = "fixture"          # token, spacing, type, missing, layout, copy, generated, fixture, rasterisation
reason = "Timeline content is the s02 demo conversation, not the board's."
decision = "accepted"       # unreviewed | accepted | fix
reviewer = "lead"

[no_capture]                # a board with no capture yet
reason = "No search states in code."
decision = "unreviewed"

[size]                      # a capture size that differs from the board
capture = "1440x900"
reason = "…"
decision = "unreviewed"
```

- A region is known when at least 95 % of its differing pixels lie inside the
  recorded regions grown by 4 px. Anything else is NEW.
- Every recorded entry counts as known whatever its decision. `accepted`
  means the difference is intended (board out of date, owner decision);
  `fix` means a confirmed defect scheduled for an alignment wave;
  `unreviewed` is waiting for a reviewer.
- `--check` exits 1 on NEW deviations only: an unrecorded region, an
  unrecorded size mismatch, a missing capture without a recorded reason, or a
  missing reference.
- `--review` writes `review.md`: unreviewed entries, NEW regions and resolved
  entries (no difference left; remove them in the change that aligned the
  screen).
- `--record` writes NEW regions into the board files as `unreviewed`
  entries (reason `UNREVIEWED`), keeps existing entries and their reasons,
  drops resolved ones and never reuses an id. It never widens an existing
  entry; a reviewer must not either.

`--classify` attributes every entry nobody decided by hand
(`src/classify.rs`): it measures the ink of both images inside the region
against the local background and sets class, decision and a one-line reason
with the measurements. Only the two lead decisions are accepted (reviewer
`lead`): the macOS traffic-light corner the headless capture cannot draw
(`chrome`), and pure glyph rasterisation where position and metrics match
within 1 design px (`rasterisation`). Confident attributions become `fix`
(reviewer `rf-design classify`): `structure` (more than 15 % of the screen),
`missing` and `extra` (ink on one side only, nothing similar within 24 px),
`token` (same geometry, other colour), `spacing` (same size, moved), `type`
(same start and height, other width; or a text height off by up to 6 px).
Everything else stays `unreviewed` with a class hint (`layout`, `copy`).
Fixture, spacing, type-scale, colour and structure differences are never
accepted. Entries a person decided are left alone.

## Tokens

```sh
cargo run -p ramflux-desktop-design --release --bin rf-design -- tokens [--out <dir>] [--check]
```

Compares by value, not by file hash:

- the palette as the application resolves it (`generated/rf_tokens.rs`
  overridden by `design/tokens/rf-tokens.json`) with the `rf-*` variables,
  light and dark;
- every value of the GPUI Kit themes (`design/generated/{light,dark}.json`)
  with the design colours of the same mode, the Kit font size with the type
  sizes and the Kit radii with the radius variables;
- Kit type and spacing utilities in production UI code (`text_xs()`,
  `gap_2()`, …) resolved at the Kit rem size, literal colours, and
  `/ 16.0` rem divisors (the rem size is 15).

`--worklist <file>` writes the W1 work list: every Kit spacing and type
utility site in production UI code grouped by file with line, the value it
renders today and the design value it should take (4 px grid; nearest design
type size), and every `/ 16.0` rem divisor.

Failing findings: value mismatches, design tokens missing in code, Kit values
and type sizes and literal colours the design does not define.
Informational: the 12 legacy `pilot-*` variables, code-only geometry tokens,
spacing utilities (the design defines no spacing scale besides `rf-gap`), rem
divisors. `design/pencil/variables.json` is the variable dump of the design
file (Pencil, read-only); refresh it when the design file changes.

## Adding a board

1. Make sure the board is in `design/boards/manifest.json` (re-import after a
   reviewed export).
2. Add a recipe in `examples/board_capture/recipes.rs`: the fixture it starts
   from, the steps that reach the state (prefer existing scenarios and
   injected `DomainEvent`s), and landmarks (element ids the screen must show).
   A dark proof or an English board reuses the recipe of its screen. A board
   with no screen gets `Plan::Skip` with the reason.
3. Capture it (`--boards <key>`), compare it, review the three-up, and record
   the reviewed deviations in `design/deviations/<key>.toml`.
4. Boards outside the priority set are captured with `--boards` and compared
   with `rf-design compare --boards`.

## CI proposal

Not added yet; the comparison needs the reference images and the macOS
renderer.

1. Light (Linux, seconds after the crate builds): the instrument's unit tests
   already run in `cargo test --workspace`. `rf-design tokens --check` becomes
   a blocking step once the three Kit type utilities are replaced (it fails
   today on `text_xs`, `text_sm`, `text_lg`).
2. Heavy (macOS, about 3 minutes of capture plus a debug build): fetch the
   reference set from a pinned artefact or release asset (never the design
   file), `refs verify --priority`, capture the priority set with `TZ=UTC`,
   `compare --check --review`, upload `report.md`, `review.md` and the
   three-up images. Non-blocking until the baseline is reviewed, then
   blocking on NEW deviations. The renderer is part of the toolchain: pin the
   macOS image and re-review the baseline from a CI run when it changes.
