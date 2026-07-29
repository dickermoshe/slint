// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore pixelfont vectorfont
use alloc::rc::Rc;
#[cfg(not(feature = "text-engine"))]
use alloc::vec::Vec;
#[cfg(not(feature = "text-engine"))]
use core::cell::RefCell;

use super::{Fixed, PhysicalLength, PhysicalSize};
use i_slint_core::graphics::BitmapFont;
#[cfg(not(feature = "text-engine"))]
use i_slint_core::graphics::FontRequest;
#[cfg(not(feature = "text-engine"))]
use i_slint_core::lengths::ScaleFactor;
#[cfg(not(feature = "text-engine"))]
use i_slint_core::textlayout::TextLayout;

#[cfg(not(feature = "text-engine"))]
i_slint_core::thread_local! {
    static BITMAP_FONTS: RefCell<Vec<&'static BitmapFont>> = RefCell::default()
}

#[cfg(not(feature = "text-engine"))]
#[derive(derive_more::From, Clone)]
pub enum GlyphAlphaMap {
    Static(&'static [u8]),
    Shared(Rc<[u8]>),
}

#[cfg(not(feature = "text-engine"))]
#[derive(Clone)]
pub struct RenderableGlyph {
    pub x: Fixed<i32, 8>,
    pub y: Fixed<i32, 8>,
    pub width: PhysicalLength,
    pub height: PhysicalLength,
    pub alpha_map: GlyphAlphaMap,
    pub pixel_stride: u16,
    pub sdf: bool,
}

#[cfg(not(feature = "text-engine"))]
impl RenderableGlyph {
    pub fn size(&self) -> PhysicalSize {
        PhysicalSize::from_lengths(self.width, self.height)
    }
}

// Subset of `RenderableGlyph`, specifically for VectorFonts.
#[cfg(feature = "text-engine")]
#[derive(Clone)]
pub struct RenderableVectorGlyph {
    pub y: Fixed<i32, 8>,
    pub width: PhysicalLength,
    pub height: PhysicalLength,
    pub alpha_map: Rc<[u8]>,
    pub pixel_stride: u16,
    pub glyph_origin_x: f32,
}

#[cfg(feature = "text-engine")]
impl RenderableVectorGlyph {
    pub fn size(&self) -> PhysicalSize {
        PhysicalSize::from_lengths(self.width, self.height)
    }
}

#[cfg(not(feature = "text-engine"))]
pub trait GlyphRenderer {
    fn render_glyph(
        &self,
        glyph_id: core::num::NonZeroU16,
        slint_context: &i_slint_core::SlintContext,
    ) -> Option<RenderableGlyph>;
    /// The amount of pixel in the original image that correspond to one pixel in the rendered image
    fn scale_delta(&self) -> Fixed<u16, 8>;
}

#[cfg(not(feature = "text-engine"))]
pub(super) use i_slint_core::textlayout::DEFAULT_FONT_SIZE;

#[cfg(not(feature = "text-engine"))]
mod pixelfont;
#[cfg(feature = "text-engine")]
pub mod vectorfont;

#[cfg(feature = "text-engine")]
pub mod systemfonts;

#[cfg(not(feature = "text-engine"))]
pub enum Font {
    PixelFont(pixelfont::PixelFont),
}

#[cfg(not(feature = "text-engine"))]
/// Returns the size of the pre-rendered font in pixels.
pub fn pixel_size(glyphs: &i_slint_core::graphics::BitmapGlyphs) -> PhysicalLength {
    PhysicalLength::new(glyphs.pixel_size)
}

#[cfg(not(feature = "text-engine"))]
impl i_slint_core::textlayout::FontMetrics<PhysicalLength> for Font {
    fn ascent(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.ascent(),
        }
    }

    fn height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.height(),
        }
    }

    fn descent(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.descent(),
        }
    }

    fn x_height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.x_height(),
        }
    }

    fn cap_height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.cap_height(),
        }
    }
}

#[cfg(not(feature = "text-engine"))]
pub fn match_font(request: &FontRequest, scale_factor: ScaleFactor) -> Font {
    let requested_weight = request
        .weight
        .and_then(|weight| weight.try_into().ok())
        .unwrap_or(/* CSS normal */ 400);

    let bitmap_font = BITMAP_FONTS.with(|fonts| {
        let fonts = fonts.borrow();

        request.family.as_ref().and_then(|requested_family| {
            fonts
                .iter()
                .filter(|bitmap_font| {
                    core::str::from_utf8(bitmap_font.family_name.as_slice()).unwrap()
                        == requested_family.as_str()
                        && bitmap_font.italic == request.italic
                })
                .min_by_key(|bitmap_font| bitmap_font.weight.abs_diff(requested_weight))
                .copied()
        })
    });

    let font = match bitmap_font {
        Some(bitmap_font) => bitmap_font,
        None => {
            if let Some(fallback_bitmap_font) = BITMAP_FONTS.with(|fonts| {
                let fonts = fonts.borrow();
                fonts
                    .iter()
                    .cloned()
                    .filter(|bitmap_font| bitmap_font.italic == request.italic)
                    .min_by_key(|bitmap_font| bitmap_font.weight.abs_diff(requested_weight))
                    .or_else(|| fonts.first().cloned())
            }) {
                fallback_bitmap_font
            } else {
                panic!(
                    "No font fallback found. The software renderer requires enabling the `EmbedForSoftwareRenderer` option when compiling slint files."
                )
            }
        }
    };

    let requested_pixel_size: PhysicalLength =
        (request.pixel_size.unwrap_or(DEFAULT_FONT_SIZE).cast() * scale_factor).cast();

    let nearest_pixel_size = font
        .glyphs
        .partition_point(|glyphs| pixel_size(glyphs) <= requested_pixel_size)
        .saturating_sub(1);
    let matching_glyphs = &font.glyphs[nearest_pixel_size];

    let pixel_size = if font.sdf { requested_pixel_size } else { pixel_size(matching_glyphs) };

    Font::PixelFont(pixelfont::PixelFont { bitmap_font: font, glyphs: matching_glyphs, pixel_size })
}

#[cfg(not(feature = "text-engine"))]
pub fn text_layout_for_font<'a, Font>(
    font: &'a Font,
    font_request: &FontRequest,
    scale_factor: ScaleFactor,
) -> TextLayout<'a, Font>
where
    Font: i_slint_core::textlayout::AbstractFont
        + i_slint_core::textlayout::TextShaper<Length = PhysicalLength>,
{
    let letter_spacing =
        font_request.letter_spacing.map(|spacing| (spacing.cast() * scale_factor).cast());

    TextLayout { font, letter_spacing }
}

pub fn register_bitmap_font(font_data: &'static BitmapFont) {
    #[cfg(not(feature = "text-engine"))]
    BITMAP_FONTS.with(|fonts| fonts.borrow_mut().push(font_data));
    #[cfg(feature = "text-engine")]
    let _ = font_data;
}
