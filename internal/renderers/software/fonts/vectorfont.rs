// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use alloc::rc::Rc;
#[cfg(feature = "embedded-ttf-only")]
use alloc::vec;
use alloc::vec::Vec;

use crate::PhysicalLength;
use crate::fixed::Fixed;
use i_slint_common::sharedfontique::fontique;

use super::RenderableVectorGlyph;

/// Number of horizontal sub-pixel positions a glyph can be placed at. The
/// shaper produces sub-pixel accurate pen positions, but glyph bitmaps live on
/// the integer pixel grid; rendering each glyph at the nearest 1/N pixel bin
/// (instead of snapping the pen to a whole pixel) keeps inter-glyph spacing
/// even. 4 bins (quarter-pixel) is enough to remove the visible unevenness at
/// UI text sizes while keeping the glyph cache small.
pub(crate) const SUBPIXEL_BIN_COUNT: i32 = 4;

/// Cache key includes blob id, font index, pixel size, glyph id, a hash of normalized
/// variation coordinates (so different variable font instances produce distinct cache
/// entries) and the horizontal sub-pixel bin.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphCacheKey {
    /// Font blob id.
    font_blob_id: u64,
    /// Font index within the blob.
    font_index: u32,
    /// Rendered pixel size.
    pixel_size: PhysicalLength,
    /// Glyph id.
    glyph_id: u16,
    /// Hash of the normalized variation coordinates.
    coords_hash: u64,
    /// Horizontal sub-pixel bin.
    subpixel_bin: u8,
    /// Faux-bold synthesis.
    embolden: bool,
    /// Faux-italic skew angle.
    skew_bits: u32,
}

pub(crate) struct RenderableGlyphWeightScale;

impl clru::WeightScale<GlyphCacheKey, RenderableVectorGlyph> for RenderableGlyphWeightScale {
    fn weight(&self, _: &GlyphCacheKey, value: &RenderableVectorGlyph) -> usize {
        value.alpha_map.len()
    }
}

pub(crate) type GlyphCache = clru::CLruCache<
    GlyphCacheKey,
    RenderableVectorGlyph,
    std::collections::hash_map::RandomState,
    RenderableGlyphWeightScale,
>;

pub(crate) fn new_glyph_cache() -> GlyphCache {
    clru::CLruCache::with_config(
        clru::CLruCacheConfig::new(core::num::NonZeroUsize::new(1024 * 1024).unwrap())
            .with_scale(RenderableGlyphWeightScale),
    )
}

pub struct VectorFont {
    font_index: u32,
    font_blob: fontique::Blob<u8>,
    #[cfg(not(feature = "embedded-ttf-only"))]
    swash_key: swash::CacheKey,
    #[cfg(not(feature = "embedded-ttf-only"))]
    swash_offset: u32,
    pixel_size: PhysicalLength,
    /// Normalized variation coordinates (F2Dot14, fvar axis order) for variable font rendering.
    #[cfg(not(feature = "embedded-ttf-only"))]
    normalized_coords: Vec<i16>,
    /// Hash of normalized_coords for use in the glyph cache key.
    coords_hash: u64,
    synthesis: fontique::Synthesis,
}

fn hash_coords(coords: &[i16]) -> u64 {
    use core::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    coords.hash(&mut hasher);
    hasher.finish()
}

impl VectorFont {
    #[cfg(not(feature = "embedded-ttf-only"))]
    fn swash_font_ref(&self) -> swash::FontRef<'_> {
        swash::FontRef {
            data: self.font_blob.data(),
            offset: self.swash_offset,
            key: self.swash_key,
        }
    }

    #[cfg(not(feature = "embedded-ttf-only"))]
    pub fn new_from_blob_and_index_with_coords(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        swash_key: swash::CacheKey,
        swash_offset: u32,
        pixel_size: PhysicalLength,
        normalized_coords: &[i16],
        synthesis: fontique::Synthesis,
    ) -> Self {
        let coords_hash = hash_coords(normalized_coords);
        Self {
            font_index,
            font_blob,
            swash_key,
            swash_offset,
            pixel_size,
            normalized_coords: normalized_coords.to_vec(),
            coords_hash,
            synthesis,
        }
    }

    #[cfg(feature = "embedded-ttf-only")]
    pub fn new_from_blob_and_index_with_coords(
        font_blob: fontique::Blob<u8>,
        font_index: u32,
        pixel_size: PhysicalLength,
        normalized_coords: &[i16],
        synthesis: fontique::Synthesis,
    ) -> Self {
        let coords_hash = hash_coords(normalized_coords);
        Self { font_index, font_blob, pixel_size, coords_hash, synthesis }
    }

    pub fn render_vector_glyph(
        &self,
        glyph_id: u16,
        subpixel_bin: u8,
        #[cfg(not(feature = "embedded-ttf-only"))] slint_context: &i_slint_core::SlintContext,
        glyph_cache: &core::cell::RefCell<GlyphCache>,
    ) -> Option<RenderableVectorGlyph> {
        let mut cache = glyph_cache.borrow_mut();

        let cache_key = GlyphCacheKey {
            font_blob_id: self.font_blob.id(),
            font_index: self.font_index,
            pixel_size: self.pixel_size,
            glyph_id,
            coords_hash: self.coords_hash,
            subpixel_bin,
            embolden: self.synthesis.embolden(),
            skew_bits: self.synthesis.skew().unwrap_or_default().to_bits(),
        };

        if let Some(entry) = cache.get(&cache_key) {
            return Some(entry.clone());
        }

        let subpixel_offset_x = subpixel_bin as f32 / SUBPIXEL_BIN_COUNT as f32;

        #[cfg(not(feature = "embedded-ttf-only"))]
        let glyph = {
            let font_ref = self.swash_font_ref();
            let mut ctx = slint_context.swash_scale_context().borrow_mut();
            let mut scaler = ctx
                .builder(font_ref)
                .size(self.pixel_size.get() as f32)
                .normalized_coords(&self.normalized_coords)
                .build();
            let mut renderer = swash::scale::Render::new(&[swash::scale::Source::Outline]);
            renderer
                .format(swash::zeno::Format::Alpha)
                .offset(swash::zeno::Vector::new(subpixel_offset_x, 0.0));
            if self.synthesis.embolden() {
                renderer.embolden(self.pixel_size.get() as f32 * 0.02);
            }
            if let Some(skew) = self.synthesis.skew() {
                renderer.transform(Some(swash::zeno::Transform::skew(
                    swash::zeno::Angle::from_degrees(skew),
                    swash::zeno::Angle::from_degrees(0.0),
                )));
            }
            let image = renderer.render(&mut scaler, glyph_id)?;
            let placement = image.placement;
            let alpha_map: Rc<[u8]> = image.data.into();

            Some(RenderableVectorGlyph {
                y: Fixed::from_integer(placement.top - placement.height as i32),
                width: PhysicalLength::new(placement.width.try_into().unwrap()),
                height: PhysicalLength::new(placement.height.try_into().unwrap()),
                alpha_map,
                pixel_stride: placement.width.try_into().unwrap(),
                glyph_origin_x: placement.left as f32,
            })
        };

        #[cfg(feature = "embedded-ttf-only")]
        let glyph = self.render_static_ttf_glyph(glyph_id, subpixel_offset_x);

        if let Some(ref glyph) = glyph {
            cache.put_with_weight(cache_key, glyph.clone()).ok();
        }
        glyph
    }

    #[cfg(feature = "embedded-ttf-only")]
    fn render_static_ttf_glyph(
        &self,
        glyph_id: u16,
        subpixel_offset_x: f32,
    ) -> Option<RenderableVectorGlyph> {
        struct Outline {
            commands: Vec<zeno::Command>,
            scale: f32,
        }

        impl Outline {
            fn point(&self, x: f32, y: f32) -> zeno::Point {
                zeno::Point::new(x * self.scale, y * self.scale)
            }
        }

        impl ttf_parser::OutlineBuilder for Outline {
            fn move_to(&mut self, x: f32, y: f32) {
                self.commands.push(zeno::Command::MoveTo(self.point(x, y)));
            }

            fn line_to(&mut self, x: f32, y: f32) {
                self.commands.push(zeno::Command::LineTo(self.point(x, y)));
            }

            fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
                let control = self.point(x1, y1);
                let point = self.point(x, y);
                self.commands.push(zeno::Command::QuadTo(control, point));
            }

            fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
                let control1 = self.point(x1, y1);
                let control2 = self.point(x2, y2);
                let point = self.point(x, y);
                self.commands.push(zeno::Command::CurveTo(control1, control2, point));
            }

            fn close(&mut self) {
                self.commands.push(zeno::Command::Close);
            }
        }

        let face = ttf_parser::Face::parse(self.font_blob.data(), self.font_index).ok()?;
        let scale = self.pixel_size.get() as f32 / face.units_per_em() as f32;
        let mut outline = Outline { commands: Vec::new(), scale };
        face.tables().glyf?.outline(ttf_parser::GlyphId(glyph_id), &mut outline)?;

        let offset = zeno::Vector::new(subpixel_offset_x, 0.0);
        let mut mask = zeno::Mask::new(outline.commands.as_slice());
        mask.format(zeno::Format::Alpha)
            .origin(zeno::Origin::BottomLeft)
            .offset(offset)
            .render_offset(offset);
        if let Some(skew) = self.synthesis.skew() {
            mask.transform(Some(zeno::Transform::skew(
                zeno::Angle::from_degrees(skew),
                zeno::Angle::from_degrees(0.0),
            )));
        }
        let (mut alpha_map, mut placement) = mask.render();
        if self.synthesis.embolden() && placement.width > 0 && placement.height > 0 {
            let old_stride = placement.width as usize;
            let new_stride = old_stride.checked_add(1)?;
            let height = placement.height as usize;
            let mut emboldened = vec![0; new_stride.checked_mul(height)?];
            for row in 0..height {
                for column in 0..old_stride {
                    let alpha = alpha_map[row * old_stride + column];
                    let destination = row * new_stride + column;
                    emboldened[destination] = emboldened[destination].max(alpha);
                    emboldened[destination + 1] =
                        emboldened[destination + 1].max(alpha.saturating_add(1) / 2);
                }
            }
            alpha_map = emboldened;
            placement.width = placement.width.checked_add(1)?;
        }
        let alpha_map: Rc<[u8]> = alpha_map.into();

        Some(RenderableVectorGlyph {
            y: Fixed::from_integer(placement.top - placement.height as i32),
            width: PhysicalLength::new(placement.width.try_into().ok()?),
            height: PhysicalLength::new(placement.height.try_into().ok()?),
            alpha_map,
            pixel_stride: placement.width.try_into().ok()?,
            glyph_origin_x: placement.left as f32,
        })
    }
}
