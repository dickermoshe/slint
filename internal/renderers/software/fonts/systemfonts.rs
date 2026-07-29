// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore fallbackfont
#[cfg(not(feature = "embedded-ttf-only"))]
use core::cell::RefCell;
#[cfg(not(feature = "embedded-ttf-only"))]
use std::collections::HashMap;

#[cfg(all(feature = "systemfonts", not(target_family = "wasm")))]
use i_slint_common::sharedfontique::fontique;
#[cfg(not(feature = "embedded-ttf-only"))]
use i_slint_common::sharedfontique::{HashedBlob, fontique};
#[cfg(not(feature = "embedded-ttf-only"))]
struct CachedFontInfo {
    swash_key: swash::CacheKey,
    swash_offset: u32,
}

#[cfg(not(feature = "embedded-ttf-only"))]
i_slint_core::thread_local! {
    // swash font info cached and indexed by fontique blob id (unique incremental) and true type collection index
    static SWASH_FONTS: RefCell<HashMap<(HashedBlob, u32), CachedFontInfo>> = Default::default();
}

#[cfg(not(feature = "embedded-ttf-only"))]
pub fn get_swash_font_info(
    blob: &fontique::Blob<u8>,
    index: u32,
) -> Option<(swash::CacheKey, u32)> {
    SWASH_FONTS.with(|font_cache| {
        let mut cache = font_cache.borrow_mut();
        let key = (blob.clone().into(), index);
        if let Some(info) = cache.get(&key) {
            return Some((info.swash_key, info.swash_offset));
        }
        let font_ref = swash::FontRef::from_index(blob.data(), index as usize)?;
        let info = CachedFontInfo { swash_key: font_ref.key, swash_offset: font_ref.offset };
        let result = (info.swash_key, info.swash_offset);
        cache.insert(key, info);
        Some(result)
    })
}

#[cfg(all(feature = "systemfonts", not(target_family = "wasm")))]
pub fn register_font_from_path(
    collection: &mut fontique::Collection,
    path: &std::path::Path,
) -> Result<(), alloc::boxed::Box<dyn std::error::Error>> {
    let requested_path = path.canonicalize().unwrap_or_else(|_| path.into());
    let contents = std::fs::read(requested_path)?;
    collection.register_fonts(contents.into(), None);
    Ok(())
}
