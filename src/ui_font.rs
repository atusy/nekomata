//! Installed faces for nekomata's widgets, found by family name.
//!
//! The platform's font matching names the candidate faces, CoreText on
//! macOS and fontconfig elsewhere, as felis-shaping does for terminals.
//! That crate keeps face bytes private and egui needs them, so this picks
//! among the candidates and reads the chosen file itself.
use std::path::{Path, PathBuf};

use swash::{FontRef, StringId, Style};

const REGULAR_WEIGHT: u16 = 400;

/// A face file's bytes and the face's index within a collection.
pub struct Face {
    pub bytes: Vec<u8>,
    pub index: u32,
}

/// felis's default font size, so the UI and an unconfigured terminal
/// start at the same size.
pub fn default_size_px() -> f32 {
    felis_render_wgpu::DEFAULT_FONT_SIZE_LOGICAL_PX
}

/// The face of `family` closest to regular upright text, as the platform
/// matches family names (ignoring case, localized names included).
///
/// Runs once at startup on the GUI thread. Only the matched family's
/// files are opened, so it takes about 2 ms. CoreText's per-process setup
/// costs a few hundred milliseconds on its first query, but by the time
/// the app is created opening the window has already paid it, so this
/// does not wait on it there.
pub fn find(family: &str) -> Result<Face, Unavailable> {
    let (path, index) = select(&platform::candidates(family)?, family)?;
    let bytes = std::fs::read(&path).map_err(|error| {
        Unavailable::Unreadable(format!("cannot read {}: {error}", path.display()))
    })?;
    Ok(Face { bytes, index })
}

/// Why no face of a family could be used, worded to follow the family's
/// name in a warning.
#[derive(Debug, PartialEq)]
pub enum Unavailable {
    /// The platform's font matching lists no face of the family.
    NotInstalled,
    /// Faces were listed but none could be used; the last failure.
    Unreadable(String),
    /// The platform's font matcher itself is unavailable.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NoMatcher(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("is not installed"),
            Self::Unreadable(reason) => write!(f, "cannot be loaded ({reason})"),
            Self::NoMatcher(reason) => write!(f, "cannot be looked up ({reason})"),
        }
    }
}

/// A face the platform matched to a family, before its file is read.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    path: PathBuf,
    face: FaceRef,
}

/// How the platform identifies a face within its file.
#[derive(Debug, Clone, PartialEq)]
enum FaceRef {
    /// fontconfig's face index; on macOS only tests list through it.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Index(u32),
    /// CoreText names faces, not their position in a collection; elsewhere
    /// only tests list through it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    PostScript(String),
}

/// Better faces sort first: one whose own family name (name ID 1) is the
/// request over a typographic-family or localized match, then upright,
/// normal width, covering Latin text, and nearest the regular weight.
/// Among equals the platform's order decides.
type Rank = (u8, bool, bool, bool, u16);

fn rank(font: &FontRef<'_>, family: &str) -> Rank {
    let named = |id| {
        font.localized_strings()
            .any(|name| name.id() == id && name.to_string().to_lowercase() == family)
    };
    let name = if named(StringId::Family) {
        0
    } else if named(StringId::TypographicFamily) {
        1
    } else {
        2
    };
    let attributes = font.attributes();
    (
        name,
        attributes.style() != Style::Normal,
        attributes.stretch() != swash::Stretch::NORMAL,
        font.charmap().map('a') == 0,
        attributes.weight().0.abs_diff(REGULAR_WEIGHT),
    )
}

fn select(candidates: &[Candidate], family: &str) -> Result<(PathBuf, u32), Unavailable> {
    let family = family.to_lowercase();
    let mut best: Option<(Rank, &Path, u32)> = None;
    let mut mapped: Option<(&Path, memmap2::Mmap)> = None;
    let mut failure = None;
    for candidate in candidates {
        let path = &candidate.path;
        if mapped.as_ref().is_none_or(|(at, _)| *at != path) {
            mapped = None;
            let file = match std::fs::File::open(path) {
                Ok(file) => file,
                Err(error) => {
                    failure = Some(format!("cannot open {}: {error}", path.display()));
                    continue;
                }
            };
            // SAFETY: installed fonts are not rewritten while mapped; a
            // truncation would at worst fault this startup read, the
            // hazard felis-shaping accepts for the same files.
            match unsafe { memmap2::Mmap::map(&file) } {
                Ok(data) => mapped = Some((path, data)),
                Err(error) => {
                    failure = Some(format!("cannot map {}: {error}", path.display()));
                    continue;
                }
            }
        }
        let Some((_, data)) = &mapped else { continue };
        let Some(collection) = swash::FontDataRef::new(data) else {
            failure = Some(format!("{} is not a font file", path.display()));
            continue;
        };
        let found = match &candidate.face {
            FaceRef::Index(index) => collection.get(*index as usize).map(|font| (*index, font)),
            FaceRef::PostScript(name) => {
                collection.fonts().enumerate().find_map(|(index, font)| {
                    font.localized_strings()
                        .any(|string| {
                            string.id() == StringId::PostScript && string.to_string() == *name
                        })
                        .then_some((index as u32, font))
                })
            }
        };
        let Some((index, font)) = found else {
            failure = Some(format!("{} lacks the listed face", path.display()));
            continue;
        };
        let rank = rank(&font, &family);
        if best.as_ref().is_none_or(|(at, ..)| rank < *at) {
            best = Some((rank, path, index));
        }
    }
    match (best, failure) {
        (Some((_, path, index)), _) => Ok((path.to_path_buf(), index)),
        (None, Some(reason)) => Err(Unavailable::Unreadable(reason)),
        (None, None) => Err(Unavailable::NotInstalled),
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFURL};
    use objc2_core_text::{
        CTFontDescriptor, kCTFontFamilyNameAttribute, kCTFontNameAttribute, kCTFontURLAttribute,
    };

    use super::{Candidate, FaceRef, Unavailable};

    /// The faces CoreText matches to `family`; it compares names ignoring
    /// case, accepts localized ones, and never substitutes another family.
    pub(super) fn candidates(family: &str) -> Result<Vec<Candidate>, Unavailable> {
        let name = CFString::from_str(family);
        // SAFETY: reading CoreText's immutable attribute-name constants.
        let (family_key, url_key, name_key) = unsafe {
            (
                kCTFontFamilyNameAttribute,
                kCTFontURLAttribute,
                kCTFontNameAttribute,
            )
        };
        let attributes = CFDictionary::<CFString, CFString>::from_slices(&[family_key], &[&name]);
        // SAFETY: the dictionary maps the family-name attribute to the
        // CFString value CoreText documents for it.
        let descriptor = unsafe { CTFontDescriptor::with_attributes(attributes.as_ref()) };
        // SAFETY: `None` asks for no mandatory attributes.
        let Some(matched) = (unsafe { descriptor.matching_font_descriptors(None) }) else {
            return Ok(Vec::new());
        };
        // SAFETY: CoreText documents the array as holding font descriptors.
        let matched = unsafe { CFRetained::cast_unchecked::<CFArray<CTFontDescriptor>>(matched) };
        let listed: Vec<_> = matched
            .iter()
            .filter_map(|descriptor| {
                // SAFETY: CoreText stores the URL as a CFURL and the name
                // as a CFString; another type fails the downcast. Hidden
                // system faces have no URL and are skipped.
                let (url, post_script) = unsafe {
                    (
                        descriptor.attribute(url_key)?.downcast::<CFURL>().ok()?,
                        descriptor
                            .attribute(name_key)?
                            .downcast::<CFString>()
                            .ok()?,
                    )
                };
                Some(Candidate {
                    path: url.to_file_path()?,
                    face: FaceRef::PostScript(post_script.to_string()),
                })
            })
            .collect();
        // Matched faces without a file are not "not installed".
        if listed.is_empty() && !matched.is_empty() {
            return Err(Unavailable::Unreadable(
                "CoreText names no font file for it".into(),
            ));
        }
        Ok(listed)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub(super) fn candidates(family: &str) -> Result<Vec<super::Candidate>, super::Unavailable> {
        super::fontconfig::candidates(None, family)
    }
}

/// Compiled on macOS for its tests; the dev shell provides libfontconfig.
#[cfg(any(not(target_os = "macos"), test))]
mod fontconfig {
    use std::ffi::{CStr, CString, OsStr, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use fontconfig_sys::constants::{FC_FAMILY, FC_FILE, FC_INDEX};
    use fontconfig_sys::{Fc, FcConfig, FcPattern, FcResultMatch};

    use super::{Candidate, FaceRef, Unavailable};

    #[cfg(target_os = "macos")]
    const SONAME: &str = "libfontconfig.1.dylib";
    #[cfg(not(target_os = "macos"))]
    const SONAME: &str = "libfontconfig.so.1";

    /// Loaded once and never unloaded, so its function pointers stay valid.
    pub(super) fn library() -> Option<&'static Fc> {
        static LIB: OnceLock<Option<Fc>> = OnceLock::new();
        LIB.get_or_init(|| {
            // SAFETY: loading runs only the library's initializers; every
            // symbol is resolved against the binding's signatures.
            unsafe { Fc::open(SONAME) }.ok()
        })
        .as_ref()
    }

    /// The faces fontconfig lists for `family` in `config` (the user's
    /// when `None`). Listing compares names ignoring case and blanks and,
    /// unlike matching, never substitutes another family.
    pub(super) fn candidates(
        config: Option<*mut FcConfig>,
        family: &str,
    ) -> Result<Vec<Candidate>, Unavailable> {
        let Some(lib) = library() else {
            return Err(Unavailable::NoMatcher(format!("{SONAME} is not loadable")));
        };
        // No installed family has a NUL in its name.
        let Ok(name) = CString::new(family) else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        // SAFETY: every pointer passed is live for its call; the pattern,
        // object set and listed set are created and destroyed here, and
        // strings read from a listed pattern are copied before the set is.
        unsafe {
            let pattern = (lib.FcPatternCreate)();
            if pattern.is_null() {
                return Err(failed());
            }
            (lib.FcPatternAddString)(pattern, FC_FAMILY.as_ptr(), name.as_ptr().cast());
            let objects = (lib.FcObjectSetCreate)();
            if objects.is_null() {
                (lib.FcPatternDestroy)(pattern);
                return Err(failed());
            }
            (lib.FcObjectSetAdd)(objects, FC_FILE.as_ptr());
            (lib.FcObjectSetAdd)(objects, FC_INDEX.as_ptr());
            let set = (lib.FcFontList)(config.unwrap_or(std::ptr::null_mut()), pattern, objects);
            (lib.FcObjectSetDestroy)(objects);
            (lib.FcPatternDestroy)(pattern);
            if set.is_null() {
                return Err(failed());
            }
            let fonts = match usize::try_from((*set).nfont) {
                Ok(len) if len > 0 && !(*set).fonts.is_null() => {
                    std::slice::from_raw_parts((*set).fonts, len)
                }
                _ => &[],
            };
            for &listed in fonts {
                if let Some(candidate) = candidate(lib, listed) {
                    found.push(candidate);
                }
            }
            (lib.FcFontSetDestroy)(set);
        }
        // fontconfig lists in no promised order; sort so ties are stable.
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }

    fn failed() -> Unavailable {
        Unavailable::NoMatcher("fontconfig could not list fonts".into())
    }

    /// # Safety
    /// `pattern` must be a live pattern.
    unsafe fn candidate(lib: &Fc, pattern: *mut FcPattern) -> Option<Candidate> {
        let mut file = std::ptr::null_mut();
        let mut index: c_int = 0;
        // SAFETY: `pattern` is live; fontconfig strings are NUL-terminated
        // and owned by the pattern, and are copied before returning.
        unsafe {
            if (lib.FcPatternGetString)(pattern, FC_FILE.as_ptr(), 0, &raw mut file)
                != FcResultMatch
                || file.is_null()
            {
                return None;
            }
            if (lib.FcPatternGetInteger)(pattern, FC_INDEX.as_ptr(), 0, &raw mut index)
                != FcResultMatch
            {
                index = 0;
            }
            let path = PathBuf::from(OsStr::from_bytes(CStr::from_ptr(file.cast()).to_bytes()));
            // A variable face's named instances share its face index in the
            // low 16 bits; egui loads the face at its default position.
            let index = (index.cast_unsigned()) & 0xffff;
            Some(Candidate {
                path,
                face: FaceRef::Index(index),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UDEV Gothic NF's faces, Bold sorted first, beside a regular face of
    /// the sibling family UDEV Gothic 35NF sorted before all of them.
    fn fixture() -> Option<(tempfile::TempDir, Vec<Candidate>)> {
        let regular = PathBuf::from(std::env::var_os("NEKOMATA_FONT")?);
        let source = regular.parent()?;
        let dir = tempfile::tempdir().unwrap();
        let mut candidates = vec![];
        for (from, to) in [
            ("UDEVGothic35NF-Regular.ttf", "0-sibling-regular.ttf"),
            ("UDEVGothicNF-Bold.ttf", "a-bold.ttf"),
            ("UDEVGothicNF-Italic.ttf", "b-italic.ttf"),
            ("UDEVGothicNF-Regular.ttf", "c-regular.ttf"),
        ] {
            let path = dir.path().join(to);
            std::fs::copy(source.join(from), &path).unwrap();
            candidates.push(Candidate {
                path,
                face: FaceRef::Index(0),
            });
        }
        Some((dir, candidates))
    }

    fn skip() {
        eprintln!("skipped: NEKOMATA_FONT is unset outside nix develop");
    }

    #[test]
    fn the_regular_upright_face_of_the_named_family_wins() {
        let Some((dir, candidates)) = fixture() else {
            return skip();
        };
        assert_eq!(
            select(&candidates, "UDev Gothic NF"),
            Ok((dir.path().join("c-regular.ttf"), 0))
        );
    }

    #[test]
    fn a_post_script_name_locates_its_face() {
        let Some((dir, _)) = fixture() else {
            return skip();
        };
        let path = dir.path().join("c-regular.ttf");
        let data = std::fs::read(&path).unwrap();
        let font = swash::FontRef::from_index(&data, 0).unwrap();
        let name = font
            .localized_strings()
            .find(|string| string.id() == StringId::PostScript)
            .unwrap()
            .to_string();
        let named = |name: &str| Candidate {
            path: path.clone(),
            face: FaceRef::PostScript(name.into()),
        };
        assert_eq!(
            select(&[named(&name)], "UDEV Gothic NF"),
            Ok((path.clone(), 0))
        );
        assert!(matches!(
            select(&[named("NoSuch-Regular")], "UDEV Gothic NF"),
            Err(Unavailable::Unreadable(reason)) if reason.ends_with("lacks the listed face")
        ));
    }

    #[test]
    fn unreadable_candidates_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "not a font").unwrap();
        let candidates = [
            Candidate {
                path: dir.path().join("missing.ttf"),
                face: FaceRef::Index(0),
            },
            Candidate {
                path: text,
                face: FaceRef::Index(0),
            },
        ];
        assert!(matches!(
            select(&candidates, "Anything"),
            Err(Unavailable::Unreadable(reason)) if reason.ends_with("is not a font file")
        ));
    }

    #[test]
    fn a_usable_face_wins_over_an_earlier_failure() {
        let Some((dir, mut candidates)) = fixture() else {
            return skip();
        };
        candidates.insert(
            0,
            Candidate {
                path: dir.path().join("missing.ttf"),
                face: FaceRef::Index(0),
            },
        );
        assert_eq!(
            select(&candidates, "UDEV Gothic NF"),
            Ok((dir.path().join("c-regular.ttf"), 0))
        );
    }

    #[test]
    fn no_candidate_means_the_family_is_not_installed() {
        assert_eq!(select(&[], "Anything"), Err(Unavailable::NotInstalled));
    }

    #[test]
    fn unusable_candidates_name_the_file_that_failed() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "not a font").unwrap();
        let candidates = [Candidate {
            path: text.clone(),
            face: FaceRef::Index(0),
        }];
        assert_eq!(
            select(&candidates, "Anything"),
            Err(Unavailable::Unreadable(format!(
                "{} is not a font file",
                text.display()
            )))
        );
        let missing = dir.path().join("missing.ttf");
        let Err(Unavailable::Unreadable(reason)) = select(
            &[Candidate {
                path: missing.clone(),
                face: FaceRef::Index(0),
            }],
            "Anything",
        ) else {
            panic!("a missing file is unreadable");
        };
        assert!(
            reason.starts_with(&format!("cannot open {}: ", missing.display())),
            "{reason}"
        );
    }

    #[test]
    fn each_reason_reads_as_a_predicate_of_the_family() {
        assert_eq!(Unavailable::NotInstalled.to_string(), "is not installed");
        assert_eq!(
            Unavailable::Unreadable("cannot open /f.ttf: denied".into()).to_string(),
            "cannot be loaded (cannot open /f.ttf: denied)"
        );
        assert_eq!(
            Unavailable::NoMatcher("libfontconfig.so.1 is not loadable".into()).to_string(),
            "cannot be looked up (libfontconfig.so.1 is not loadable)"
        );
    }

    #[test]
    fn fontconfig_lists_a_family_ignoring_case() {
        let Some((dir, _)) = fixture() else {
            return skip();
        };
        let Some(lib) = fontconfig::library() else {
            // The dev shell and package build provide the library and set
            // this, so a missing one fails there instead of passing.
            assert!(
                std::env::var_os("NEKOMATA_REQUIRE_FONTCONFIG").is_none(),
                "libfontconfig is not loadable"
            );
            eprintln!("skipped: libfontconfig is not loadable");
            return;
        };
        let path = std::ffi::CString::new(dir.path().as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: the config is created, filled and destroyed here.
        unsafe {
            let config = (lib.FcConfigCreate)();
            assert!((lib.FcConfigAppFontAddDir)(config, path.as_ptr().cast()) != 0);
            let candidates = fontconfig::candidates(Some(config), "udev gothic nf").unwrap();
            (lib.FcConfigDestroy)(config);
            let names: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.path.file_name().unwrap().to_owned())
                .collect();
            assert_eq!(names, ["a-bold.ttf", "b-italic.ttf", "c-regular.ttf"]);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn coretext_matches_a_bundled_family_ignoring_case() {
        let candidates = platform::candidates("hiragino kaku gothic pron").unwrap();
        assert!(!candidates.is_empty());
        assert!(
            candidates
                .iter()
                .all(|candidate| matches!(candidate.face, FaceRef::PostScript(_)))
        );
        assert!(find("hiragino kaku gothic pron").is_ok());
        assert_eq!(platform::candidates("No Such Family"), Ok(vec![]));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn coretext_matches_a_localized_family_name() {
        // Hiragino ships with macOS; its Japanese name is a localized one.
        let english = find("Hiragino Kaku Gothic ProN");
        let japanese = find("ヒラギノ角ゴ ProN");
        assert!(english.is_ok());
        assert_eq!(
            japanese.map(|face| face.index).ok(),
            english.map(|face| face.index).ok()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_hidden_system_family_is_not_reported_as_missing() {
        // CoreText matches the system UI family but names no file for it.
        assert_eq!(
            find(".AppleSystemUIFont").err(),
            Some(Unavailable::Unreadable(
                "CoreText names no font file for it".into()
            ))
        );
    }

    #[test]
    fn the_default_size_is_felis_default_font_size() {
        assert_eq!(
            default_size_px(),
            felis_render_wgpu::DEFAULT_FONT_SIZE_LOGICAL_PX
        );
    }
}
