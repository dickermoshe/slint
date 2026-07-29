// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use crate::diagnostics::BuildDiagnostics;
use crate::expression_tree::{BuiltinFunction, Expression, Unit};
use crate::object_tree::Document;
use i_slint_common::sharedfontique::{self, fontique};
use std::collections::{HashMap, HashSet};

/// The fontique collection shared by font packaging and SVG text rasterization.
#[cfg(feature = "renderer-software")]
pub struct FontCollection {
    pub collection: sharedfontique::Collection,
    pub custom_font_paths: HashMap<fontique::FamilyId, std::path::PathBuf>,
    pub custom_font_packages: HashMap<std::path::PathBuf, Vec<u8>>,
    pub custom_font_order: Vec<std::path::PathBuf>,
}

/// The lazily initialized font collection shared by the software-renderer passes.
#[cfg(feature = "renderer-software")]
pub type SharedFontCollection = std::sync::Arc<
    std::sync::LazyLock<
        std::sync::Mutex<FontCollection>,
        Box<dyn FnOnce() -> std::sync::Mutex<FontCollection> + Send + Sync>,
    >,
>;

/// Reads and validates every imported OpenType font.
#[cfg(feature = "renderer-software")]
pub fn read_custom_fonts<'a>(
    all_docs: impl Iterator<Item = &'a Document>,
    diag: &mut BuildDiagnostics,
) -> Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)> {
    let mut fonts = Vec::new();
    let mut seen_paths = HashSet::new();
    for doc in all_docs {
        for (font_path, import_token) in &doc.custom_fonts {
            if !seen_paths.insert(font_path.clone()) {
                continue;
            }
            match std::fs::read(font_path.as_str()) {
                Err(error) => {
                    diag.push_error(format!("Error loading font: {error}"), import_token);
                }
                Ok(bytes) => match sharedfontique::SoftwareFontPackage::build(bytes.as_slice()) {
                    Ok(package) => {
                        fonts.push((font_path.as_str().into(), bytes, package));
                    }
                    Err(error) => {
                        diag.push_error(
                            format!("Cannot package font '{}': {error}", font_path.as_str()),
                            import_token,
                        );
                    }
                },
            }
        }
    }
    fonts
}

/// Adds imported fonts to a collection that also provides compiler-host system fonts.
#[cfg(feature = "renderer-software")]
pub fn shared_font_collection(
    custom_fonts: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
) -> SharedFontCollection {
    let init: Box<dyn FnOnce() -> std::sync::Mutex<FontCollection> + Send + Sync> =
        Box::new(move || {
            let mut collection = sharedfontique::create_collection(true);
            let mut custom_font_paths = HashMap::new();
            let mut custom_font_packages = HashMap::new();
            let mut custom_font_order = Vec::new();
            for (path, bytes, package) in custom_fonts {
                let registered = collection.register_fonts(bytes.into(), None);
                for (family_id, _) in &registered {
                    custom_font_paths.insert(*family_id, path.clone());
                }
                custom_font_order.push(path.clone());
                custom_font_packages.insert(path, package);
            }
            std::sync::Mutex::new(FontCollection {
                collection,
                custom_font_paths,
                custom_font_packages,
                custom_font_order,
            })
        });
    std::sync::Arc::new(std::sync::LazyLock::new(init))
}

/// Packages complete OpenType fonts for the software renderer.
///
/// The package keeps all glyph and shaping tables so runtime strings do not depend on
/// string literals discovered by the compiler.
#[cfg(not(target_arch = "wasm32"))]
pub fn embed_font_packages(
    doc: &Document,
    font_collection: &SharedFontCollection,
    diag: &mut BuildDiagnostics,
) {
    use crate::diagnostics::Spanned;

    let generic_diag_location = doc.node.as_ref().map(|node| node.to_source_location());
    let mut shared = match font_collection.lock() {
        Ok(shared) => shared,
        Err(_) => {
            diag.push_error(
                "internal error: font collection lock is poisoned".into(),
                &generic_diag_location,
            );
            return;
        }
    };
    let FontCollection { collection, custom_font_paths, custom_font_packages, custom_font_order } =
        &mut *shared;

    let default_fonts: Vec<(std::path::PathBuf, fontique::QueryFont)> =
        if !collection.default_fonts.is_empty() {
            collection.default_fonts.as_ref().clone()
        } else {
            let mut fonts = Vec::new();
            for component in doc.exported_roots() {
                let (family, source_location) = component
                    .root_element
                    .borrow()
                    .bindings
                    .get("default-font-family")
                    .and_then(|binding| match &binding.borrow().expression {
                        Expression::StringLiteral(family) => {
                            Some((Some(family.clone()), binding.borrow().span.clone()))
                        }
                        _ => None,
                    })
                    .unwrap_or_default();

                let font = {
                    let mut query = collection.query();
                    query.set_families(
                        family
                            .as_ref()
                            .map(|family| fontique::QueryFamily::from(family.as_str()))
                            .into_iter()
                            .chain(
                                sharedfontique::FALLBACK_FAMILIES
                                    .into_iter()
                                    .map(fontique::QueryFamily::Generic),
                            ),
                    );
                    let mut font = None;
                    query.matches_with(|queried_font| {
                        font = Some(queried_font.clone());
                        fontique::QueryStatus::Stop
                    });
                    font
                };

                let Some(font) = font else {
                    if let Some(source_location) = source_location {
                        diag.push_error_with_span(
                            "could not find a font for the specified default family".into(),
                            source_location,
                        );
                    } else {
                        diag.push_error(
                            "could not determine a default sans-serif font".into(),
                            &generic_diag_location,
                        );
                    }
                    continue;
                };

                let path = custom_font_paths
                    .get(&font.family.0)
                    .cloned()
                    .or_else(|| {
                        #[cfg(feature = "renderer-software-system-fonts")]
                        {
                            collection.family(font.family.0)?.fonts().iter().find_map(|info| {
                                if info.index() != font.index {
                                    return None;
                                }
                                match &info.source().kind {
                                    fontique::SourceKind::Path(path) => {
                                        Some(std::path::PathBuf::from(path.as_ref()))
                                    }
                                    fontique::SourceKind::Memory(_) => None,
                                }
                            })
                        }
                        #[cfg(not(feature = "renderer-software-system-fonts"))]
                        {
                            None
                        }
                    })
                    .unwrap_or_else(|| std::path::PathBuf::from("<memory font>"));
                fonts.push((path, font));
            }
            fonts
        };

    let mut packages_to_emit = Vec::new();
    for path in custom_font_order.iter() {
        if let Some(package) = custom_font_packages.get(path) {
            packages_to_emit.push((path.clone(), package.clone()));
        }
    }
    for (path, font) in &default_fonts {
        let package = custom_font_packages.get(path).cloned().or_else(|| {
            match sharedfontique::SoftwareFontPackage::build(font.blob.data()) {
                Ok(package) => Some(package),
                Err(error) => {
                    diag.push_error(
                        format!("Cannot package font '{}': {error}", path.display()),
                        &generic_diag_location,
                    );
                    None
                }
            }
        });
        if let Some(package) = package {
            packages_to_emit.push((path.clone(), package));
        }
    }

    let mut packaged_hashes = HashSet::new();
    let mut emit_package = |path: &std::path::Path, package: Vec<u8>| {
        let parsed = match sharedfontique::SoftwareFontPackage::parse(&package) {
            Ok(parsed) => parsed,
            Err(error) => {
                diag.push_error(
                    format!("Cannot validate font package '{}': {error}", path.display()),
                    &generic_diag_location,
                );
                return;
            }
        };
        if !packaged_hashes.insert((parsed.hash, parsed.font_data.len())) {
            return;
        }

        let resource_id = doc
            .embedded_file_resources
            .borrow_mut()
            .push_and_get_key(crate::embedded_resources::EmbeddedResources {
            path: Some(path.to_string_lossy().as_ref().into()),
            kind: crate::embedded_resources::EmbeddedResourcesKind::SoftwareRendererFontPackageData(
                package,
            ),
        });
        for component in doc.exported_roots() {
            component.init_code.borrow_mut().font_registration_code.push(
                Expression::FunctionCall {
                    function: BuiltinFunction::RegisterCustomFontByMemory.into(),
                    arguments: vec![Expression::NumberLiteral(resource_id.0 as _, Unit::None)],
                    source_location: None,
                },
            );
        }
    };

    for (path, package) in packages_to_emit {
        emit_package(&path, package);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn embed_font_packages(
    _doc: &Document,
    _font_collection: &SharedFontCollection,
    _diag: &mut BuildDiagnostics,
) {
}
