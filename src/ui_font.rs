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
pub fn find(family: &str) -> Option<Face> {
    let chosen = locate(family)?;
    let bytes = match std::sync::Arc::try_unwrap(chosen.bytes) {
        Ok(FontBytes::Read(bytes)) => bytes,
        Ok(mapped) => mapped.as_ref().to_vec(),
        Err(shared) => shared.as_ref().as_ref().to_vec(),
    };
    Some(Face {
        bytes,
        index: chosen.index,
    })
}

/// The face [`find`] would copy, with the very bytes it was chosen from, for a
/// caller that keeps a mapped font as it is, as for fonts too large to copy.
pub(crate) fn locate(family: &str) -> Option<Chosen> {
    select(&platform::candidates(family), family).map(|(_, chosen)| chosen)
}

/// The face chosen for a family and the bytes it was chosen from, so that a
/// font rewritten after the choice cannot hand its caller other contents.
pub(crate) struct Chosen {
    pub(crate) index: u32,
    pub(crate) bytes: std::sync::Arc<FontBytes>,
}

/// Whether nothing rewrites `file` while the system runs, which holds for a
/// file on a mounted APFS snapshot, such as macOS's sealed system volume.
/// Neither a path nor a read-only mount proves it: `/System/Volumes/Data` is
/// writable, a read-only mount can be a network share, a bind mount over a
/// writable source or an unsealed system volume an administrator can remount
/// writable, and a Nix store path can be made writable by the store's owner.
#[cfg(target_os = "macos")]
fn immutable_font(file: &std::fs::File) -> bool {
    use std::os::fd::AsRawFd as _;
    let mut volume = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `file` is open for the call and `volume` is written in full on
    // success, the only case in which it is read.
    unsafe {
        libc::fstatfs(file.as_raw_fd(), volume.as_mut_ptr()) == 0
            && volume.assume_init().f_flags & libc::MNT_SNAPSHOT as u32 != 0
    }
}

#[cfg(not(target_os = "macos"))]
fn immutable_font(_file: &std::fs::File) -> bool {
    false
}

/// A font file's bytes, mapped when nothing can rewrite the file and read
/// otherwise, since a file rewritten under a live map breaks the map's
/// contract and a truncation faults whoever reads it next.
pub(crate) enum FontBytes {
    Mapped(memmap2::Mmap),
    Read(Vec<u8>),
}

impl AsRef<[u8]> for FontBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Mapped(map) => map,
            Self::Read(bytes) => bytes,
        }
    }
}

/// The bytes of the font file at `path`; see [`FontBytes`].
pub(crate) fn load(path: &Path) -> Option<FontBytes> {
    let mut file = std::fs::File::open(path).ok()?;
    if immutable_font(&file) {
        // SAFETY: nothing rewrites the file (see `immutable_font`), so the
        // bytes stay valid for as long as the map lives.
        return unsafe { memmap2::Mmap::map(&file) }
            .ok()
            .map(FontBytes::Mapped);
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).ok()?;
    Some(FontBytes::Read(bytes))
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

/// The best candidate's file and the face chosen from it.
fn select(candidates: &[Candidate], family: &str) -> Option<(PathBuf, Chosen)> {
    let family = family.to_lowercase();
    let mut best: Option<(Rank, &Path, u32, std::sync::Arc<FontBytes>)> = None;
    let mut loaded: Option<(&Path, std::sync::Arc<FontBytes>)> = None;
    for candidate in candidates {
        if loaded
            .as_ref()
            .is_none_or(|(path, _)| *path != candidate.path)
        {
            let Some(data) = load(&candidate.path) else {
                continue;
            };
            loaded = Some((&candidate.path, std::sync::Arc::new(data)));
        }
        let Some((_, data)) = &loaded else { continue };
        let Some(collection) = swash::FontDataRef::new(data.as_ref().as_ref()) else {
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
        let Some((index, font)) = found else { continue };
        let rank = rank(&font, &family);
        if best.as_ref().is_none_or(|(at, ..)| rank < *at) {
            best = Some((rank, &candidate.path, index, data.clone()));
        }
    }
    best.map(|(_, path, index, bytes)| (path.to_path_buf(), Chosen { index, bytes }))
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFURL};
    use objc2_core_text::{
        CTFontDescriptor, kCTFontFamilyNameAttribute, kCTFontNameAttribute, kCTFontURLAttribute,
    };

    use super::{Candidate, FaceRef};

    /// The faces CoreText matches to `family`; it compares names ignoring
    /// case, accepts localized ones, and never substitutes another family.
    pub(super) fn candidates(family: &str) -> Vec<Candidate> {
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
            return Vec::new();
        };
        // SAFETY: CoreText documents the array as holding font descriptors.
        let matched = unsafe { CFRetained::cast_unchecked::<CFArray<CTFontDescriptor>>(matched) };
        matched
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
            .collect()
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub(super) fn candidates(family: &str) -> Vec<super::Candidate> {
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

    use super::{Candidate, FaceRef};

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
    pub(super) fn candidates(config: Option<*mut FcConfig>, family: &str) -> Vec<Candidate> {
        let Some(lib) = library() else {
            return Vec::new();
        };
        let Ok(name) = CString::new(family) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        // SAFETY: every pointer passed is live for its call; the pattern,
        // object set and listed set are created and destroyed here, and
        // strings read from a listed pattern are copied before the set is.
        unsafe {
            let pattern = (lib.FcPatternCreate)();
            if pattern.is_null() {
                return found;
            }
            (lib.FcPatternAddString)(pattern, FC_FAMILY.as_ptr(), name.as_ptr().cast());
            let objects = (lib.FcObjectSetCreate)();
            if objects.is_null() {
                (lib.FcPatternDestroy)(pattern);
                return found;
            }
            (lib.FcObjectSetAdd)(objects, FC_FILE.as_ptr());
            (lib.FcObjectSetAdd)(objects, FC_INDEX.as_ptr());
            let set = (lib.FcFontList)(config.unwrap_or(std::ptr::null_mut()), pattern, objects);
            (lib.FcObjectSetDestroy)(objects);
            (lib.FcPatternDestroy)(pattern);
            if set.is_null() {
                return found;
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
        found
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
            select(&candidates, "UDev Gothic NF").map(|(path, chosen)| (path, chosen.index)),
            Some((dir.path().join("c-regular.ttf"), 0))
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
            select(&[named(&name)], "UDEV Gothic NF").map(|(path, chosen)| (path, chosen.index)),
            Some((path.clone(), 0))
        );
        assert!(select(&[named("NoSuch-Regular")], "UDEV Gothic NF").is_none());
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
        assert!(select(&candidates, "Anything").is_none());
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
            let candidates = fontconfig::candidates(Some(config), "udev gothic nf");
            (lib.FcConfigDestroy)(config);
            let names: Vec<_> = candidates
                .iter()
                .map(|candidate| candidate.path.file_name().unwrap().to_owned())
                .collect();
            assert_eq!(names, ["a-bold.ttf", "b-italic.ttf", "c-regular.ttf"]);
        }
    }

    #[test]
    fn fontconfig_locates_a_color_emoji_family() {
        let Some(font) = std::env::var_os("NEKOMATA_COLOR_EMOJI_FONT") else {
            eprintln!("skipped: NEKOMATA_COLOR_EMOJI_FONT is unset outside nix develop");
            return;
        };
        let Some(lib) = fontconfig::library() else {
            assert!(
                std::env::var_os("NEKOMATA_REQUIRE_FONTCONFIG").is_none(),
                "libfontconfig is not loadable"
            );
            eprintln!("skipped: libfontconfig is not loadable");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let emoji = dir.path().join("emoji.ttf");
        std::fs::copy(font, &emoji).unwrap();
        let path = std::ffi::CString::new(dir.path().as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: the config is created, filled and destroyed here.
        let candidates = unsafe {
            let config = (lib.FcConfigCreate)();
            assert!((lib.FcConfigAppFontAddDir)(config, path.as_ptr().cast()) != 0);
            let candidates = fontconfig::candidates(Some(config), "Noto Color Emoji");
            (lib.FcConfigDestroy)(config);
            candidates
        };
        // A face without Latin letters still matches its family.
        let (path, chosen) = select(&candidates, "Noto Color Emoji").unwrap();
        assert_eq!((path, chosen.index), (emoji.clone(), 0));
        // The bytes come from the selection itself, so a font rewritten after
        // it was chosen cannot hand the caller other contents.
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&emoji, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&emoji, b"").unwrap();
        assert!(swash::FontRef::from_index(chosen.bytes.as_ref().as_ref(), 0).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn coretext_matches_a_bundled_family_ignoring_case() {
        let candidates = platform::candidates("hiragino kaku gothic pron");
        assert!(!candidates.is_empty());
        assert!(
            candidates
                .iter()
                .all(|candidate| matches!(candidate.face, FaceRef::PostScript(_)))
        );
        assert!(find("hiragino kaku gothic pron").is_some());
        assert!(platform::candidates("No Such Family").is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn coretext_matches_a_localized_family_name() {
        // Hiragino ships with macOS; its Japanese name is a localized one.
        let english = find("Hiragino Kaku Gothic ProN");
        let japanese = find("ヒラギノ角ゴ ProN");
        assert!(english.is_some());
        assert_eq!(
            japanese.map(|face| face.index),
            english.map(|face| face.index)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn coretext_locates_a_color_emoji_font_without_reading_it() {
        // A face without Latin letters still matches its family.
        let (path, _) = select(
            &platform::candidates("Apple Color Emoji"),
            "Apple Color Emoji",
        )
        .unwrap();
        assert_eq!(path.file_name().unwrap(), "Apple Color Emoji.ttc");
    }

    #[test]
    fn fonts_that_can_change_are_read() {
        use std::io::Write as _;
        let mut writable = tempfile::NamedTempFile::new().unwrap();
        writable.write_all(b"font bytes").unwrap();
        let bytes = load(writable.path()).unwrap();
        assert!(matches!(bytes, FontBytes::Read(_)));
        assert_eq!(bytes.as_ref(), b"font bytes");
        // A store path can be made writable or lie under a writable mount, so
        // only the sealed system volume proves a font cannot change.
        match std::env::var_os("NEKOMATA_COLOR_EMOJI_FONT") {
            Some(font) => assert!(matches!(load(Path::new(&font)), Some(FontBytes::Read(_)))),
            None => eprintln!("skipped store check: NEKOMATA_COLOR_EMOJI_FONT is unset"),
        }
        assert!(load(&writable.path().with_extension("missing")).is_none());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_sealed_system_volume_keeps_apple_color_emoji_mapped() {
        // Ask mount(8), not the check under test, whether / is sealed.
        let Ok(mounts) = std::process::Command::new("/sbin/mount").output() else {
            eprintln!("skipped: /sbin/mount cannot run here");
            return;
        };
        let sealed = String::from_utf8_lossy(&mounts.stdout)
            .lines()
            .any(|line| line.contains(" on / (") && line.contains("sealed"));
        let chosen = locate("Apple Color Emoji").unwrap();
        assert_eq!(
            matches!(*chosen.bytes, FontBytes::Mapped(_)),
            sealed,
            "a font on the sealed system volume is mapped, and read elsewhere"
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
