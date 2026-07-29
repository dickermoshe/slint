// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use super::{fontique, skrifa};
use core::fmt;
use core::ops::RangeInclusive;
use std::sync::Arc;

/// Magic bytes at the start of a software-renderer font package.
pub const SOFTWARE_FONT_PACKAGE_MAGIC: [u8; 8] = *b"SLFNTPKG";

/// Current software-renderer font package format version.
pub const SOFTWARE_FONT_PACKAGE_VERSION: u16 = 1;

const HEADER_LEN: usize = 40;
const FACE_FIXED_LEN: usize = 48;

const SCRIPT_ARABIC: u16 = 1 << 0;
const SCRIPT_HEBREW: u16 = 1 << 1;
const SCRIPT_DEVANAGARI: u16 = 1 << 2;

/// Error returned while building or reading a software-renderer font package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareFontPackageError {
    InvalidFont(String),
    InvalidPackage(&'static str),
    UnsupportedVersion(u16),
}

impl fmt::Display for SoftwareFontPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFont(reason) => write!(formatter, "invalid font: {reason}"),
            Self::InvalidPackage(reason) => write!(formatter, "invalid font package: {reason}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported font package version {version}")
            }
        }
    }
}

impl std::error::Error for SoftwareFontPackageError {}

/// Metadata for a variable-font axis stored in a software-renderer font package.
#[derive(Debug, Clone, PartialEq)]
pub struct SoftwareFontAxis {
    pub tag: [u8; 4],
    pub minimum: f32,
    pub default: f32,
    pub maximum: f32,
}

/// Metadata for one face stored in a software-renderer font package.
#[derive(Debug, Clone, PartialEq)]
pub struct SoftwareFontFace {
    pub index: u32,
    pub family_name: String,
    pub weight: f32,
    pub width: f32,
    pub style: fontique::FontStyle,
    pub units_per_em: u16,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub coverage: Vec<RangeInclusive<u32>>,
    pub axes: Vec<SoftwareFontAxis>,
    script_flags: u16,
}

impl SoftwareFontFace {
    pub fn covers_arabic(&self) -> bool {
        self.script_flags & SCRIPT_ARABIC != 0
    }

    pub fn covers_hebrew(&self) -> bool {
        self.script_flags & SCRIPT_HEBREW != 0
    }

    pub fn covers_devanagari(&self) -> bool {
        self.script_flags & SCRIPT_DEVANAGARI != 0
    }

    pub fn covers(&self, codepoint: u32) -> bool {
        self.coverage
            .binary_search_by(|range| {
                if codepoint < *range.start() {
                    core::cmp::Ordering::Greater
                } else if codepoint > *range.end() {
                    core::cmp::Ordering::Less
                } else {
                    core::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// A validated software-renderer font package borrowed from generated static data.
#[derive(Debug)]
pub struct SoftwareFontPackage<'a> {
    pub faces: Vec<SoftwareFontFace>,
    pub font_data: &'a [u8],
    pub hash: u64,
}

impl SoftwareFontPackage<'_> {
    /// Validates a font and builds a deterministic package containing the complete input bytes.
    pub fn build(font_data: &[u8]) -> Result<Vec<u8>, SoftwareFontPackageError> {
        use skrifa::MetadataProvider as _;

        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            system_fonts: false,
            ..Default::default()
        });
        let registered =
            collection.register_fonts(fontique::Blob::new(Arc::new(font_data.to_vec())), None);
        if registered.is_empty() {
            return Err(SoftwareFontPackageError::InvalidFont(
                "the file contains no supported OpenType faces".into(),
            ));
        }

        let mut faces = Vec::new();
        for (family_id, font_infos) in registered {
            let family_name = collection
                .family_name(family_id)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    SoftwareFontPackageError::InvalidFont(
                        "a face does not contain a family name".into(),
                    )
                })?
                .to_owned();

            for info in font_infos {
                let face =
                    skrifa::FontRef::from_index(font_data, info.index()).map_err(|error| {
                        SoftwareFontPackageError::InvalidFont(format!(
                            "face {} cannot be parsed: {error}",
                            info.index()
                        ))
                    })?;
                let charmap = face.charmap();
                if !charmap.has_map() {
                    return Err(SoftwareFontPackageError::InvalidFont(format!(
                        "face {} does not contain a supported Unicode cmap",
                        info.index()
                    )));
                }
                if face.outline_glyphs().format().is_none() {
                    return Err(SoftwareFontPackageError::InvalidFont(format!(
                        "face {} does not contain supported glyph outlines",
                        info.index()
                    )));
                }

                let coverage = coverage_ranges(charmap.mappings().map(|(codepoint, _)| codepoint));
                if coverage.is_empty() {
                    return Err(SoftwareFontPackageError::InvalidFont(format!(
                        "face {} has an empty Unicode cmap",
                        info.index()
                    )));
                }

                let script_flags = script_flags(&coverage);
                let axes = face
                    .axes()
                    .iter()
                    .map(|axis| SoftwareFontAxis {
                        tag: axis.tag().to_be_bytes(),
                        minimum: axis.min_value(),
                        default: axis.default_value(),
                        maximum: axis.max_value(),
                    })
                    .collect();
                let metrics = face.metrics(
                    skrifa::instance::Size::unscaled(),
                    skrifa::instance::LocationRef::default(),
                );

                faces.push(SoftwareFontFace {
                    index: info.index(),
                    family_name: family_name.clone(),
                    weight: info.weight().value(),
                    width: info.width().ratio(),
                    style: info.style(),
                    units_per_em: metrics.units_per_em,
                    ascent: metrics.ascent,
                    descent: metrics.descent,
                    leading: metrics.leading,
                    coverage,
                    axes,
                    script_flags,
                });
            }
        }
        faces.sort_by_key(|face| face.index);

        let manifest = encode_manifest(&faces)?;
        let font_len: u32 = font_data.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font data exceeds the package size limit".into())
        })?;
        let manifest_len: u32 = manifest.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont(
                "font metadata exceeds the package size limit".into(),
            )
        })?;
        let face_count: u32 = faces.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font contains too many faces".into())
        })?;
        let hash = stable_hash(font_data);

        let mut package = Vec::with_capacity(HEADER_LEN + manifest.len() + font_data.len());
        package.extend_from_slice(&SOFTWARE_FONT_PACKAGE_MAGIC);
        push_u16(&mut package, SOFTWARE_FONT_PACKAGE_VERSION);
        push_u16(&mut package, HEADER_LEN as u16);
        push_u32(&mut package, face_count);
        push_u32(&mut package, manifest_len);
        push_u32(&mut package, font_len);
        push_u64(&mut package, hash);
        push_u64(&mut package, stable_hash(&manifest));
        package.extend_from_slice(&manifest);
        package.extend_from_slice(font_data);
        Ok(package)
    }
}

impl<'a> SoftwareFontPackage<'a> {
    /// Parses and validates a package without trusting offsets from generated data.
    pub fn parse(package: &'a [u8]) -> Result<Self, SoftwareFontPackageError> {
        if package.len() < HEADER_LEN {
            return Err(SoftwareFontPackageError::InvalidPackage("truncated header"));
        }
        if package[..8] != SOFTWARE_FONT_PACKAGE_MAGIC {
            return Err(SoftwareFontPackageError::InvalidPackage("invalid magic"));
        }

        let version = read_u16(package, 8)?;
        if version != SOFTWARE_FONT_PACKAGE_VERSION {
            return Err(SoftwareFontPackageError::UnsupportedVersion(version));
        }
        let header_len = read_u16(package, 10)? as usize;
        if header_len != HEADER_LEN {
            return Err(SoftwareFontPackageError::InvalidPackage("invalid header size"));
        }
        let face_count = read_u32(package, 12)? as usize;
        let manifest_len = read_u32(package, 16)? as usize;
        let font_len = read_u32(package, 20)? as usize;
        let expected_hash = read_u64(package, 24)?;
        let expected_manifest_hash = read_u64(package, 32)?;
        let manifest_end = header_len
            .checked_add(manifest_len)
            .ok_or(SoftwareFontPackageError::InvalidPackage("manifest size overflow"))?;
        let font_end = manifest_end
            .checked_add(font_len)
            .ok_or(SoftwareFontPackageError::InvalidPackage("font size overflow"))?;
        if font_end != package.len() {
            return Err(SoftwareFontPackageError::InvalidPackage("inconsistent package size"));
        }
        if face_count == 0 || face_count > manifest_len / FACE_FIXED_LEN {
            return Err(SoftwareFontPackageError::InvalidPackage("invalid face count"));
        }

        let manifest = &package[header_len..manifest_end];
        if stable_hash(manifest) != expected_manifest_hash {
            return Err(SoftwareFontPackageError::InvalidPackage("manifest hash mismatch"));
        }
        let faces = decode_manifest(manifest, face_count)?;
        let font_data = &package[manifest_end..font_end];
        if stable_hash(font_data) != expected_hash {
            return Err(SoftwareFontPackageError::InvalidPackage("font data hash mismatch"));
        }
        for face in &faces {
            skrifa::FontRef::from_index(font_data, face.index).map_err(|_| {
                SoftwareFontPackageError::InvalidPackage("face index does not match font data")
            })?;
        }

        Ok(Self { faces, font_data, hash: expected_hash })
    }

    /// Returns true when the bytes begin with the package magic.
    pub fn has_magic(data: &[u8]) -> bool {
        data.starts_with(&SOFTWARE_FONT_PACKAGE_MAGIC)
    }
}

fn encode_manifest(faces: &[SoftwareFontFace]) -> Result<Vec<u8>, SoftwareFontPackageError> {
    let mut manifest = Vec::new();
    for face in faces {
        let family = face.family_name.as_bytes();
        let family_len: u16 = family.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font family name is too long".into())
        })?;
        let coverage_len: u32 = face.coverage.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font has too many coverage ranges".into())
        })?;
        let axes_len: u16 = face.axes.len().try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font has too many variation axes".into())
        })?;
        let (style, oblique_angle) = match face.style {
            fontique::FontStyle::Normal => (0, f32::NAN),
            fontique::FontStyle::Italic => (1, f32::NAN),
            fontique::FontStyle::Oblique(angle) => (2, angle.unwrap_or(f32::NAN)),
        };
        let record_len = FACE_FIXED_LEN
            .checked_add(family.len())
            .and_then(|len| len.checked_add(face.coverage.len() * 8))
            .and_then(|len| len.checked_add(face.axes.len() * 16))
            .ok_or_else(|| {
                SoftwareFontPackageError::InvalidFont("font face metadata is too large".into())
            })?;
        let record_len: u32 = record_len.try_into().map_err(|_| {
            SoftwareFontPackageError::InvalidFont("font face metadata is too large".into())
        })?;

        push_u32(&mut manifest, record_len);
        push_u32(&mut manifest, face.index);
        push_u16(&mut manifest, family_len);
        manifest.push(style);
        manifest.push(0);
        push_u16(&mut manifest, face.script_flags);
        push_u16(&mut manifest, face.units_per_em);
        push_f32(&mut manifest, face.weight);
        push_f32(&mut manifest, face.width);
        push_f32(&mut manifest, face.ascent);
        push_f32(&mut manifest, face.descent);
        push_f32(&mut manifest, face.leading);
        push_u32(&mut manifest, coverage_len);
        push_u16(&mut manifest, axes_len);
        push_u16(&mut manifest, 0);
        push_f32(&mut manifest, oblique_angle);
        manifest.extend_from_slice(family);
        for range in &face.coverage {
            push_u32(&mut manifest, *range.start());
            push_u32(&mut manifest, *range.end());
        }
        for axis in &face.axes {
            manifest.extend_from_slice(&axis.tag);
            push_f32(&mut manifest, axis.minimum);
            push_f32(&mut manifest, axis.default);
            push_f32(&mut manifest, axis.maximum);
        }
    }
    Ok(manifest)
}

fn decode_manifest(
    manifest: &[u8],
    face_count: usize,
) -> Result<Vec<SoftwareFontFace>, SoftwareFontPackageError> {
    let mut faces = Vec::with_capacity(face_count);
    let mut offset = 0;
    for _ in 0..face_count {
        let record_len = read_u32(manifest, offset)? as usize;
        if record_len < FACE_FIXED_LEN {
            return Err(SoftwareFontPackageError::InvalidPackage("invalid face record size"));
        }
        let record_end = offset
            .checked_add(record_len)
            .ok_or(SoftwareFontPackageError::InvalidPackage("face record size overflow"))?;
        if record_end > manifest.len() {
            return Err(SoftwareFontPackageError::InvalidPackage("truncated face record"));
        }

        let index = read_u32(manifest, offset + 4)?;
        let family_len = read_u16(manifest, offset + 8)? as usize;
        let style_kind = *manifest
            .get(offset + 10)
            .ok_or(SoftwareFontPackageError::InvalidPackage("truncated face style"))?;
        let script_flags = read_u16(manifest, offset + 12)?;
        let units_per_em = read_u16(manifest, offset + 14)?;
        let weight = read_f32(manifest, offset + 16)?;
        let width = read_f32(manifest, offset + 20)?;
        let ascent = read_f32(manifest, offset + 24)?;
        let descent = read_f32(manifest, offset + 28)?;
        let leading = read_f32(manifest, offset + 32)?;
        let coverage_len = read_u32(manifest, offset + 36)? as usize;
        let axes_len = read_u16(manifest, offset + 40)? as usize;
        let oblique_angle = read_f32(manifest, offset + 44)?;
        let style = match style_kind {
            0 if oblique_angle.is_nan() => fontique::FontStyle::Normal,
            1 if oblique_angle.is_nan() => fontique::FontStyle::Italic,
            2 if oblique_angle.is_nan() => fontique::FontStyle::Oblique(None),
            2 if oblique_angle.is_finite() => fontique::FontStyle::Oblique(Some(oblique_angle)),
            _ => return Err(SoftwareFontPackageError::InvalidPackage("invalid face style")),
        };

        if units_per_em == 0
            || !weight.is_finite()
            || !width.is_finite()
            || !ascent.is_finite()
            || !descent.is_finite()
            || !leading.is_finite()
            || script_flags & !(SCRIPT_ARABIC | SCRIPT_HEBREW | SCRIPT_DEVANAGARI) != 0
        {
            return Err(SoftwareFontPackageError::InvalidPackage("invalid face metadata"));
        }

        let family_start = offset + FACE_FIXED_LEN;
        let family_end = family_start
            .checked_add(family_len)
            .ok_or(SoftwareFontPackageError::InvalidPackage("family name size overflow"))?;
        let coverage_end = family_end
            .checked_add(
                coverage_len
                    .checked_mul(8)
                    .ok_or(SoftwareFontPackageError::InvalidPackage("coverage size overflow"))?,
            )
            .ok_or(SoftwareFontPackageError::InvalidPackage("coverage size overflow"))?;
        let axes_end = coverage_end
            .checked_add(
                axes_len
                    .checked_mul(16)
                    .ok_or(SoftwareFontPackageError::InvalidPackage("axes size overflow"))?,
            )
            .ok_or(SoftwareFontPackageError::InvalidPackage("axes size overflow"))?;
        if axes_end != record_end {
            return Err(SoftwareFontPackageError::InvalidPackage(
                "face record fields do not match its size",
            ));
        }

        let family_name = core::str::from_utf8(&manifest[family_start..family_end])
            .map_err(|_| {
                SoftwareFontPackageError::InvalidPackage("family name is not valid UTF-8")
            })?
            .to_owned();
        if family_name.is_empty() {
            return Err(SoftwareFontPackageError::InvalidPackage("empty family name"));
        }
        let mut coverage = Vec::with_capacity(coverage_len);
        for range_index in 0..coverage_len {
            let range_offset = family_end + range_index * 8;
            let start = read_u32(manifest, range_offset)?;
            let end = read_u32(manifest, range_offset + 4)?;
            if start > end
                || end > char::MAX as u32
                || coverage
                    .last()
                    .is_some_and(|previous: &RangeInclusive<u32>| *previous.end() >= start)
            {
                return Err(SoftwareFontPackageError::InvalidPackage(
                    "coverage ranges are not sorted",
                ));
            }
            coverage.push(start..=end);
        }
        let mut axes = Vec::with_capacity(axes_len);
        for axis_index in 0..axes_len {
            let axis_offset = coverage_end + axis_index * 16;
            let axis = SoftwareFontAxis {
                tag: manifest[axis_offset..axis_offset + 4]
                    .try_into()
                    .map_err(|_| SoftwareFontPackageError::InvalidPackage("truncated axis tag"))?,
                minimum: read_f32(manifest, axis_offset + 4)?,
                default: read_f32(manifest, axis_offset + 8)?,
                maximum: read_f32(manifest, axis_offset + 12)?,
            };
            if !axis.minimum.is_finite()
                || !axis.default.is_finite()
                || !axis.maximum.is_finite()
                || axis.minimum > axis.default
                || axis.default > axis.maximum
            {
                return Err(SoftwareFontPackageError::InvalidPackage(
                    "invalid variation axis metadata",
                ));
            }
            axes.push(axis);
        }
        faces.push(SoftwareFontFace {
            index,
            family_name,
            weight,
            width,
            style,
            units_per_em,
            ascent,
            descent,
            leading,
            coverage,
            axes,
            script_flags,
        });
        offset = record_end;
    }
    if offset != manifest.len() {
        return Err(SoftwareFontPackageError::InvalidPackage("unused manifest bytes"));
    }
    Ok(faces)
}

fn coverage_ranges(codepoints: impl IntoIterator<Item = u32>) -> Vec<RangeInclusive<u32>> {
    let mut ranges = Vec::new();
    let mut current: Option<(u32, u32)> = None;
    for codepoint in codepoints {
        match current {
            Some((start, end)) if codepoint == end.saturating_add(1) => {
                current = Some((start, codepoint));
            }
            Some((start, end)) => {
                ranges.push(start..=end);
                current = Some((codepoint, codepoint));
            }
            None => current = Some((codepoint, codepoint)),
        }
    }
    if let Some((start, end)) = current {
        ranges.push(start..=end);
    }
    ranges
}

fn script_flags(coverage: &[RangeInclusive<u32>]) -> u16 {
    let covers_range = |script_range: RangeInclusive<u32>| {
        coverage.iter().any(|range| {
            *range.start() <= *script_range.end() && *script_range.start() <= *range.end()
        })
    };
    let mut flags = 0;
    if [
        0x0600..=0x06ff,
        0x0750..=0x077f,
        0x08a0..=0x08ff,
        0xfb50..=0xfdff,
        0xfe70..=0xfeff,
        0x1ee00..=0x1eeff,
    ]
    .into_iter()
    .any(covers_range)
    {
        flags |= SCRIPT_ARABIC;
    }
    if covers_range(0x0590..=0x05ff) || covers_range(0xfb1d..=0xfb4f) {
        flags |= SCRIPT_HEBREW;
    }
    if covers_range(0x0900..=0x097f) || covers_range(0xa8e0..=0xa8ff) {
        flags |= SCRIPT_DEVANAGARI;
    }
    flags
}

fn stable_hash(data: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    data.iter().fold(OFFSET_BASIS, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(PRIME))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(output: &mut Vec<u8>, value: f32) {
    push_u32(output, value.to_bits());
}

fn read_array<const N: usize>(
    data: &[u8],
    offset: usize,
) -> Result<[u8; N], SoftwareFontPackageError> {
    data.get(offset..offset.saturating_add(N))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(SoftwareFontPackageError::InvalidPackage("truncated numeric field"))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, SoftwareFontPackageError> {
    Ok(u16::from_le_bytes(read_array(data, offset)?))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, SoftwareFontPackageError> {
    Ok(u32::from_le_bytes(read_array(data, offset)?))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, SoftwareFontPackageError> {
    Ok(u64::from_le_bytes(read_array(data, offset)?))
}

fn read_f32(data: &[u8], offset: usize) -> Result<f32, SoftwareFontPackageError> {
    Ok(f32::from_bits(read_u32(data, offset)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inter_font() -> &'static [u8] {
        include_bytes!("Inter-VariableFont.ttf")
    }

    fn table_record_offset(font: &[u8], tag: [u8; 4]) -> usize {
        let table_count = u16::from_be_bytes(font[4..6].try_into().unwrap()) as usize;
        (0..table_count)
            .map(|index| 12 + index * 16)
            .find(|offset| font[*offset..*offset + 4] == tag)
            .unwrap()
    }

    #[test]
    fn package_round_trip_preserves_complete_font() {
        let package = SoftwareFontPackage::build(inter_font()).unwrap();
        let parsed = SoftwareFontPackage::parse(&package).unwrap();

        assert_eq!(parsed.font_data, inter_font());
        assert!(!parsed.faces.is_empty());
        assert!(parsed.faces.iter().any(|face| face.family_name == "Inter"));
        assert!(parsed.faces.iter().any(|face| face.covers(u32::from('A'))));
        assert!(parsed.faces.iter().any(|face| !face.axes.is_empty()));
    }

    #[test]
    fn package_output_is_deterministic() {
        assert_eq!(
            SoftwareFontPackage::build(inter_font()).unwrap(),
            SoftwareFontPackage::build(inter_font()).unwrap()
        );
    }

    #[test]
    fn package_rejects_corrupted_data() {
        let mut package = SoftwareFontPackage::build(inter_font()).unwrap();
        *package.last_mut().unwrap() ^= 1;
        assert_eq!(
            SoftwareFontPackage::parse(&package).unwrap_err(),
            SoftwareFontPackageError::InvalidPackage("font data hash mismatch")
        );
    }

    #[test]
    fn package_rejects_corrupted_manifest() {
        let mut package = SoftwareFontPackage::build(inter_font()).unwrap();
        package[HEADER_LEN + FACE_FIXED_LEN] ^= 1;
        assert_eq!(
            SoftwareFontPackage::parse(&package).unwrap_err(),
            SoftwareFontPackageError::InvalidPackage("manifest hash mismatch")
        );
    }

    #[test]
    fn package_rejects_impossible_face_count_before_allocating() {
        let mut package = SoftwareFontPackage::build(inter_font()).unwrap();
        package[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            SoftwareFontPackage::parse(&package).unwrap_err(),
            SoftwareFontPackageError::InvalidPackage("invalid face count")
        );
    }

    #[test]
    fn package_rejects_unknown_version() {
        let mut package = SoftwareFontPackage::build(inter_font()).unwrap();
        package[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            SoftwareFontPackage::parse(&package).unwrap_err(),
            SoftwareFontPackageError::UnsupportedVersion(2)
        );
    }

    #[test]
    fn package_builder_rejects_missing_cmap() {
        let mut font = inter_font().to_vec();
        let cmap_record = table_record_offset(&font, *b"cmap");
        font[cmap_record..cmap_record + 4].copy_from_slice(b"xxxx");

        assert!(matches!(
            SoftwareFontPackage::build(&font),
            Err(SoftwareFontPackageError::InvalidFont(_))
        ));
    }

    #[test]
    fn package_builder_rejects_invalid_table_offset() {
        let mut font = inter_font().to_vec();
        let cmap_record = table_record_offset(&font, *b"cmap");
        font[cmap_record + 8..cmap_record + 12].copy_from_slice(&u32::MAX.to_be_bytes());

        assert!(matches!(
            SoftwareFontPackage::build(&font),
            Err(SoftwareFontPackageError::InvalidFont(_))
        ));
    }

    #[test]
    fn package_builder_rejects_missing_outlines() {
        let mut font = inter_font().to_vec();
        let glyf_record = table_record_offset(&font, *b"glyf");
        font[glyf_record..glyf_record + 4].copy_from_slice(b"xxxx");

        assert!(matches!(
            SoftwareFontPackage::build(&font),
            Err(SoftwareFontPackageError::InvalidFont(_))
        ));
    }

    #[test]
    fn package_records_complex_script_coverage() {
        let arabic = SoftwareFontPackage::build(include_bytes!(
            "../../../tests/screenshots/fonts/NotoSansArabic-Variable.ttf"
        ))
        .unwrap();
        let arabic = SoftwareFontPackage::parse(&arabic).unwrap();
        assert!(arabic.faces.iter().any(SoftwareFontFace::covers_arabic));
        assert!(arabic.faces.iter().any(|face| face.covers(0x0644) && !face.axes.is_empty()));

        let hebrew = SoftwareFontPackage::build(include_bytes!(
            "../../../tests/screenshots/fonts/NotoSansHebrew-Variable.ttf"
        ))
        .unwrap();
        let hebrew = SoftwareFontPackage::parse(&hebrew).unwrap();
        assert!(hebrew.faces.iter().any(SoftwareFontFace::covers_hebrew));
        assert!(hebrew.faces.iter().any(|face| face.covers(0x05b0)));

        let devanagari = SoftwareFontPackage::build(include_bytes!(
            "../../../tests/screenshots/fonts/NotoSansDevanagari-Variable.ttf"
        ))
        .unwrap();
        let devanagari = SoftwareFontPackage::parse(&devanagari).unwrap();
        assert!(devanagari.faces.iter().any(SoftwareFontFace::covers_devanagari));
        assert!(devanagari.faces.iter().any(|face| face.covers(0x093f)));
    }
}
