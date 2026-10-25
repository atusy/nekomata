//! Installed faces for nekomata's widgets, found by family name.
//!
//! felis-shaping resolves terminal families but keeps the face bytes
//! private, and egui needs them, so this reads the font directories itself.
use std::path::{Path, PathBuf};

use swash::{FontDataRef, StringId, Style};

/// Applied when the platform reports no UI text size.
const FALLBACK_SIZE_PX: f32 = 15.0;
const REGULAR_WEIGHT: u16 = 400;
/// Font trees nest a few levels (`/usr/share/fonts/truetype/<vendor>`);
/// the bound only stops a symlink cycle.
const MAX_DEPTH: usize = 8;

/// A face file's bytes and the face's index within a collection.
pub struct Face {
    pub bytes: Vec<u8>,
    pub index: u32,
}

/// The platform's UI text size, or 15 px where the platform reports none.
pub fn default_size_px() -> f32 {
    system_size_px().unwrap_or(FALLBACK_SIZE_PX)
}

#[cfg(target_os = "macos")]
fn system_size_px() -> Option<f32> {
    use objc2_core_text::{CTFont, CTFontUIFontType};
    // SAFETY: `None` asks for the user's own language.
    let font = unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, 0.0, None) }?;
    // SAFETY: `font` is a live CTFont.
    let size = unsafe { font.size() } as f32;
    (size > 0.0).then_some(size)
}

#[cfg(not(target_os = "macos"))]
fn system_size_px() -> Option<f32> {
    None
}

/// The upright face nearest the regular weight in `family`, compared
/// ignoring case as CoreText and fontconfig do.
pub fn find(family: &str) -> Option<Face> {
    find_in(&system_dirs(), family)
}

fn find_in(dirs: &[PathBuf], family: &str) -> Option<Face> {
    let family = family.to_lowercase();
    let mut files = Vec::new();
    for dir in dirs {
        collect_files(dir, 0, &mut files);
    }
    let mut best: Option<(u32, PathBuf, u32)> = None;
    for path in files {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        // SAFETY: installed fonts are not rewritten while mapped; a
        // truncation would at worst fault this startup scan, the hazard
        // felis-shaping accepts for the same files.
        let Ok(data) = (unsafe { memmap2::Mmap::map(&file) }) else {
            continue;
        };
        let Some(collection) = FontDataRef::new(&data) else {
            continue;
        };
        for (index, font) in collection.fonts().enumerate() {
            let named = font.localized_strings().any(|name| {
                matches!(name.id(), StringId::Family | StringId::TypographicFamily)
                    && name.to_string().to_lowercase() == family
            });
            if !named {
                continue;
            }
            let attributes = font.attributes();
            let distance = u32::from(attributes.weight().0.abs_diff(REGULAR_WEIGHT))
                + if attributes.style() == Style::Normal {
                    0
                } else {
                    10_000
                };
            if best.as_ref().is_none_or(|(at, ..)| distance < *at) {
                best = Some((distance, path.clone(), index as u32));
            }
        }
    }
    let (_, path, index) = best?;
    Some(Face {
        bytes: std::fs::read(path).ok()?,
        index,
    })
}

fn collect_files(path: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.is_file() {
        let font = path.extension().is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        });
        if font {
            out.push(path.to_path_buf());
        }
    } else if metadata.is_dir() && depth < MAX_DEPTH {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for entry in entries {
            collect_files(&entry, depth + 1, out);
        }
    }
}

#[cfg(target_os = "macos")]
fn system_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join("Library/Fonts"));
    }
    dirs
}

#[cfg(not(target_os = "macos"))]
fn system_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .into_iter()
        .chain(std::env::split_paths(&data_dirs))
        .map(|dir| dir.join("fonts"))
        .chain(home.map(|home| home.join(".fonts")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> Option<tempfile::TempDir> {
        let font = std::env::var_os("NEKOMATA_FONT")?;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("truetype");
        std::fs::create_dir(&nested).unwrap();
        std::fs::copy(&font, nested.join("UDEVGothicNF-Regular.ttf")).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a font").unwrap();
        Some(dir)
    }

    #[test]
    fn a_family_is_found_ignoring_case_in_nested_directories() {
        let Some(dir) = fixture_dir() else {
            eprintln!("skipped: NEKOMATA_FONT is unset outside nix develop");
            return;
        };
        let face = find_in(&[dir.path().to_path_buf()], "UDev Gothic NF").unwrap();
        assert_eq!(face.index, 0);
        assert!(FontDataRef::new(&face.bytes).is_some());
    }

    #[test]
    fn an_unknown_family_or_directory_finds_nothing() {
        let dir = fixture_dir().unwrap_or_else(|| tempfile::tempdir().unwrap());
        let dirs = [dir.path().to_path_buf(), dir.path().join("missing")];
        assert!(find_in(&dirs, "No Such Family").is_none());
    }

    #[test]
    fn the_default_size_is_positive() {
        assert!(default_size_px() > 0.0);
    }
}
