<!--
Copyright © SixtyFPS GmbH <info@slint.dev>
SPDX-License-Identifier: MIT
-->

# Embedded font generation

The embedded font is derived from `tests/screenshots/fonts/NotoSansHebrew-Variable.ttf`.
FontTools fixes all variation axes and retains the OpenType layout closure required for Hebrew
mark positioning.

Generate temporary static fonts:

```sh
fonttools varLib.instancer NotoSansHebrew-Variable.ttf \
    wght=400 --static --no-recalc-timestamp \
    --output NotoSansHebrew-400-Static.ttf
```

Generate the embedded fonts:

```sh
fonttools subset NotoSansHebrew-400-Static.ttf \
    --output-file=NotoSansHebrew-ASCII-Embedded.ttf \
    --unicodes=U+0020-007E,U+0590-05FF,U+2000-206F,U+FB1D-FB4F \
    --layout-features='*' \
    --name-IDs=1,2,4,6,16,17 --name-languages=0x409 \
    --notdef-glyph --notdef-outline --recommended-glyphs \
    --no-recalc-timestamp
```

The adjacent `.license` file identifies the upstream Noto project and the OFL-1.1 license.
