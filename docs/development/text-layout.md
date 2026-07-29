# Text layout system

> Note for AI coding assistants (agents):
> **When to load this document:** Working on `internal/core/textlayout.rs`,
> `internal/core/textlayout/`, `internal/core/styled_text.rs`, text rendering,
> line breaking, or font handling.
> For general build commands and project structure, see `/AGENTS.md`.

## Overview

The standard-library software renderer uses one display-text pipeline for plain,
styled, and input text:

```text
UTF-8 text
    -> paragraph and bidi analysis
    -> font matching and cluster-safe fallback
    -> OpenType shaping
    -> line breaking, wrapping, alignment, and elision
    -> glyph-ID rasterization
    -> software pixel blending
```

Parley coordinates paragraph analysis, bidi, shaping, fallback, and line layout.
Harfrust performs OpenType shaping through Parley. Fontique matches font faces and
fallbacks. Skrifa reads font metadata, and Swash rasterizes positioned glyph IDs.

The same Parley layout feeds measurement and painting. Do not introduce a
script-specific renderer or a scalar-to-glyph shortcut for display text.

## Key files

| File | Purpose |
|------|---------|
| `internal/core/textlayout/sharedparley.rs` | Unified shaping, layout, measurement, hit geometry, and painting |
| `internal/common/sharedfontique.rs` | Shared Fontique collection and hosted-font configuration |
| `internal/common/sharedfontique/font_package.rs` | Versioned embedded-font package and bounds-checked parser |
| `internal/compiler/passes/embed_glyphs.rs` | Compile-time font validation, packaging, and generated registration |
| `internal/renderers/software/fonts/vectorfont.rs` | Swash glyph-ID rasterization and renderer-owned glyph caching |
| `internal/renderers/software/fonts/systemfonts.rs` | Swash face cache and optional hosted font-path registration |
| `internal/renderers/software/lib.rs` | Software paint integration |
| `internal/common/styled_text.rs` | Markdown and HTML spans, link ranges, and paragraph data |

The older files under `internal/core/textlayout/` still support the legacy
no-standard-library bitmap-font configuration. They are not the standard
software renderer's display-text path.

## Embedded fonts

Applications continue importing fonts from `.slint` files and selecting
`EmbedForSoftwareRenderer` in `slint-build`.

For that configuration, the compiler:

1. Reads each imported OpenType file once.
2. Validates every face, Unicode character map, and outline format.
3. Records family, style, weight, width, metrics, variation axes, coverage
   ranges, and selected script coverage.
4. Stores the complete original font bytes in a versioned package.
5. Emits font registration before initial component layout.

Keeping the complete font preserves GSUB, GPOS, GDEF, outlines, metrics,
variations, and hinting for arbitrary run-time strings. The package parser
checks all lengths and offsets and verifies the font-data hash before
registration.

Package metadata is stable Slint-owned data. The parser checks separate hashes
for the manifest and original font data. Do not serialize Fontique, Parley,
Harfrust, Skrifa, or Swash objects.

## Font matching and fallback

`FontContext::register_static_font()` accepts both packaged and raw static fonts.
Packaged coverage metadata determines which application families enter each
script fallback chain.

Fallback order is:

1. The requested named family.
2. The closest face in that family.
3. Packaged application families in declaration order.
4. Hosted fonts when system discovery is enabled.
5. The selected face's `.notdef` glyph.

Fontique and Parley keep fallback cluster-safe. A selected `FontData`, face
index, variation coordinates, and synthesis settings remain attached to each
shaped run through rasterization.

ESP-IDF builds enable the in-memory text engine without Fontique's `system`
feature. They do not scan environment paths or the file system. Desktop
backends enable system discovery separately.

## Paragraph layout

`create_text_paragraphs()` splits explicit newlines without changing source byte
ranges. `LayoutWithoutLineBreaksBuilder` applies:

- requested family, size, weight, style, and letter spacing;
- styled spans and link brushes;
- word, character, or no-wrap behavior;
- zero font size for bidi formatting controls, which keeps them in bidi
  analysis without painting glyphs.

Parley applies Unicode bidi and OpenType shaping. Visual runs retain logical
cluster ranges. Paragraph direction uses the Unicode first-strong rule, with
neutral-only text remaining left-to-right.

Alignment has distinct logical and physical forms:

- `start` and `end` use the paragraph direction;
- `left` and `right` use physical edges;
- `center` uses the physical center.

`text_size()`, `text_content_widths()`, and painting all consume Parley layout
results. Wrapping and elision operate on shaped cluster advances, never Unicode
scalar widths.

## Painting and caching

`GlyphRenderer::draw_glyph_run()` receives:

- the resolved font blob and face index;
- effective pixel size;
- normalized variation coordinates;
- synthesis parameters;
- positioned glyph IDs, advances, and offsets.

The normal software renderer passes glyph IDs to Swash.
It does not map Unicode characters again during painting.

Flash-constrained targets can select
`renderer-software-embedded-ttf-only`.
This profile keeps the same Parley layout and passes the same positioned glyph
IDs to a small `ttf-parser` and Zeno rasterizer.
It accepts only static TrueType `glyf` outlines and does not include hinting,
CFF, CFF2, variable-font, color-font, or bitmap-font rasterization.
Faux italic uses an outline transform, and faux bold expands the rendered alpha
mask.
Unsupported or malformed faces return no glyph bitmap without panicking.

The glyph cache belongs to `SoftwareRenderer`. Its key includes font identity,
face index, glyph ID, pixel size, variation coordinates, subpixel position, and
synthesis settings. Scale changes invalidate text layout caches. Renderer
ownership makes the cache measurable and allows full-buffer and render-by-line
painting to share it.

## Elision

Elision is calculated once for the complete final visible line after visual
layout. `Layout::line_elision()`:

1. Detects the overflowing visual edge.
2. Reserves the independently shaped indicator width.
3. Finds a retained interval at visual-cluster boundaries.
4. Uses the paragraph inline-end edge if both edges overflow.

The indicator preference is U+2026, then three periods. Glyph ID zero rejects an
indicator candidate. If neither candidate is available, text is truncated and a
deduplicated diagnostic is emitted.

Painting applies the retained interval to glyphs, decorations, inline-code
backgrounds, and link hit geometry. The indicator is painted once on the
line's baseline.

## Text input boundary

Text input uses the same layout and glyph painting engine for visual
consistency. Existing editing logic still owns caret movement, selection
affinity, deletion, IME behavior, and other editing semantics. Do not treat
those editing algorithms as a second display-text renderer.

## Tests

Focused commands:

```sh
cargo test -p i-slint-common --features shared-fontique font_package
cargo test -p i-slint-core --lib textlayout::sharedparley::tests
cargo check -p i-slint-renderer-software --no-default-features --features std
cargo check -p i-slint-renderer-software --no-default-features --features std,swash-rasterizer
cargo check -p i-slint-renderer-software --no-default-features --features std,embedded-ttf-only
```

The complex-script screenshot case is
`tests/screenshots/cases/text/complex-script-layout.slint`. It covers Arabic,
Hebrew, Devanagari, mixed-direction text, marks, variable weight, wrapping, and
elision in full-buffer and render-by-line software rendering.

The static TrueType-only profile has a separate ASCII and Hebrew reference at
`tests/screenshots/cases/text/embedded-ttf-hebrew.slint`.
Run it with:

```sh
cargo test --manifest-path tests/Cargo.toml -p test-driver-screenshots \
    --no-default-features --features software-embedded-ttf-only \
    embedded_ttf_hebrew
```

When changing shaping or fallback, compare glyph IDs, logical cluster ranges,
advances, and offsets. When changing layout, verify that measurement and paint
use the same result and that no wrap or elision boundary splits a Parley
cluster.
