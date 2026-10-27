//! Validated OSC 8 targets opened outside the UI thread.
use eframe::egui;
use felis_client_core::{ActivationTarget, SchemeClass};
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Bounds a launcher that never exits, so one stuck opener cannot keep the
/// pane busy forever.
const OPENER_TIMEOUT: Duration = Duration::from_secs(10);

const BUSY: &str = "Still opening the previous link; click again when it finishes";

#[derive(Default)]
pub struct LinkGesture(Option<ActivationTarget>);

impl LinkGesture {
    pub fn active(&self) -> bool {
        self.0.is_some()
    }

    pub fn press(
        &mut self,
        target: Option<ActivationTarget>,
        modifier: bool,
        terminal_captured: bool,
    ) {
        if modifier && !terminal_captured {
            self.0 = target;
        }
    }

    pub fn release(
        &mut self,
        target: Option<ActivationTarget>,
        clicked: bool,
    ) -> Option<ActivationTarget> {
        let pressed = self.0.take()?;
        (clicked && target.is_some_and(|target| target.as_str() == pressed.as_str()))
            .then_some(pressed)
    }
}

#[derive(Default)]
pub struct LinkOpener {
    queued: Option<ActivationTarget>,
    task: Option<tokio::task::JoinHandle<()>>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
    pub error: Option<String>,
}

impl LinkOpener {
    /// Queues `target` for the next tick. While a launcher still runs the
    /// click is refused visibly rather than deferred: a deferred launch could
    /// fire up to `OPENER_TIMEOUT` later, after the user has moved on.
    pub fn request(&mut self, target: ActivationTarget) {
        if self.task.is_some() {
            self.error = Some(BUSY.into());
        } else {
            self.queued = Some(target);
            self.error = None;
        }
    }

    pub fn tick(&mut self, runtime: &tokio::runtime::Runtime, ctx: &egui::Context) {
        if let Some(result) = self.result.lock().unwrap().take() {
            self.task = None;
            self.error = result.err();
        }
        let Some(target) = self.queued.take() else {
            return;
        };
        let result = self.result.clone();
        let ctx = ctx.clone();
        self.task = Some(runtime.spawn(async move {
            let opened = open(target).await;
            *result.lock().unwrap() = Some(opened);
            ctx.request_repaint();
        }));
    }
}

impl Drop for LinkOpener {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub fn modifier(modifiers: egui::Modifiers, macos: bool) -> bool {
    !modifiers.alt
        && if macos {
            modifiers.mac_cmd && !modifiers.ctrl
        } else {
            modifiers.ctrl && !modifiers.mac_cmd
        }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    MacOs,
    Linux,
}

impl Platform {
    fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// One launcher invocation: the program followed by its arguments.
type Step = Vec<OsString>;

/// What activating a target does. A `file:` target is revealed rather than
/// opened: a link label can claim `https://…` while its target is a script
/// or application bundle, and handing that to the default application would
/// execute it.
#[derive(Debug, PartialEq, Eq)]
enum Launch {
    Open(OsString),
    Reveal {
        /// The canonical path of the linked file.
        item: PathBuf,
        /// The directory that contains `item`; the Linux fallback opens it.
        folder: PathBuf,
    },
}

impl Launch {
    fn verb(&self) -> &'static str {
        match self {
            Self::Open(_) => "open the link",
            Self::Reveal { .. } => "show the linked file",
        }
    }
}

/// The hover hint for an activatable target.
pub fn hint(target: &ActivationTarget, macos: bool) -> &'static str {
    match (target.scheme() == SchemeClass::File, macos) {
        (true, true) => "Cmd+click to reveal in Finder",
        (true, false) => "Ctrl+click to show in the file manager",
        (false, true) => "Cmd+click to open",
        (false, false) => "Ctrl+click to open",
    }
}

/// Converts a `file:` URI to a local absolute path. `local_host` is this
/// machine's hostname, which `ls --hyperlink` and shell integrations put in
/// the authority.
fn file_path(uri: &str, local_host: &OsStr) -> Result<PathBuf, String> {
    let rest = uri
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file:"))
        .map(|_| &uri[5..])
        .ok_or("Not a file link")?;
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let path = match rest.strip_prefix("//") {
        Some(authority) => {
            let split = authority.find('/').unwrap_or(authority.len());
            let (host, path) = authority.split_at(split);
            let local = host.is_empty()
                || host.eq_ignore_ascii_case("localhost")
                || local_host
                    .to_str()
                    .is_some_and(|local| host.eq_ignore_ascii_case(local));
            if !local {
                return Err(format!("The linked file is on another host ({host})"));
            }
            path
        }
        None => rest,
    };
    if !path.starts_with('/') {
        return Err("The file link has no absolute path".into());
    }
    let bytes = percent_decode(path).ok_or("The file link has invalid percent-encoding")?;
    if bytes.contains(&0) {
        return Err("The file link contains a NUL byte".into());
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    Some(bytes)
}

/// A `file://` URI whose path is percent-encoded outside the RFC 3986
/// unreserved set and `/`, so it needs no quoting inside GVariant text.
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// Resolves the target on the filesystem. Canonicalizing first means the
/// folder is the real parent: `/x/evil.sh/..` cannot make the folder the
/// script itself, and a symlink cannot point the folder at a file.
fn resolve(target: &ActivationTarget) -> Result<Launch, String> {
    if target.scheme() != SchemeClass::File {
        return Ok(Launch::Open(target.as_str().into()));
    }
    let path = file_path(target.as_str(), &gethostname::gethostname())?;
    let item = std::fs::canonicalize(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "The linked file does not exist".to_owned(),
        _ => format!("Could not find the linked file: {error}"),
    })?;
    let folder = item.parent().unwrap_or(&item).to_owned();
    if !std::fs::metadata(&folder).is_ok_and(|metadata| metadata.is_dir()) {
        return Err("The linked file has no containing folder".into());
    }
    Ok(Launch::Reveal { item, folder })
}

/// Launchers to try in order until one succeeds. Pure so every platform's
/// argv is testable on any host.
fn plan(platform: Platform, launch: &Launch) -> Vec<Step> {
    match (platform, launch) {
        (Platform::MacOs, Launch::Open(uri)) => vec![vec!["/usr/bin/open".into(), uri.clone()]],
        (Platform::Linux, Launch::Open(uri)) => vec![vec!["xdg-open".into(), uri.clone()]],
        (Platform::MacOs, Launch::Reveal { item, .. }) => {
            vec![vec!["/usr/bin/open".into(), "-R".into(), item.into()]]
        }
        // The freedesktop file manager interface selects the item without
        // opening it. Without that service, opening the folder is the
        // closest equivalent that never runs the file.
        (Platform::Linux, Launch::Reveal { item, folder }) => vec![
            [
                "gdbus",
                "call",
                "--session",
                "--timeout",
                "5",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("['{}']", file_uri(item)),
                "''",
            ]
            .map(OsString::from)
            .to_vec(),
            vec!["xdg-open".into(), folder.into()],
        ],
    }
}

fn command(step: &[OsString]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&step[0]);
    command.args(&step[1..]).kill_on_drop(true);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

async fn open(target: ActivationTarget) -> Result<(), String> {
    let platform = Platform::current().ok_or("Opening links is not supported on this platform")?;
    let launch = resolve(&target)?;
    let mut outcome = Ok(());
    for step in plan(platform, &launch) {
        outcome = run(&step, launch.verb()).await;
        if outcome.is_ok() {
            break;
        }
    }
    outcome
}

async fn run(step: &[OsString], verb: &str) -> Result<(), String> {
    match tokio::time::timeout(OPENER_TIMEOUT, command(step).status()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!(
            "Could not {verb} ({status}); check the default application for this type"
        )),
        Ok(Err(error)) => Err(format!("Could not start the link opener: {error}")),
        Err(_) => Err(format!(
            "Timed out trying to {verb}; check the default application for this type"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_gestures_keep_the_press_owner_through_modifier_changes() {
        let target = || ActivationTarget::parse("https://example.org/").unwrap();
        let mut gesture = LinkGesture::default();
        gesture.press(Some(target()), false, false);
        // Adding the modifier only on release cannot steal a TUI click.
        assert!(gesture.release(Some(target()), true).is_none());
        gesture.press(Some(target()), true, true);
        assert!(!gesture.active());
        gesture.press(Some(target()), true, false);
        assert!(gesture.active());
        // Release needs no modifier: ownership was fixed at press time.
        assert!(gesture.release(Some(target()), true).is_some());
        assert!(!gesture.active());
        gesture.press(Some(target()), true, false);
        assert!(gesture.release(None, false).is_none());
        assert!(!gesture.active());
    }

    fn parse(uri: &str) -> ActivationTarget {
        ActivationTarget::parse(uri).unwrap()
    }

    fn resolved(uri: &str) -> Result<Launch, String> {
        resolve(&parse(uri))
    }

    #[test]
    fn opener_preserves_the_validated_uri_as_one_literal_argument() {
        let target = parse("https://example.org/?q=$(echo%20x)&a='hello'");
        let launch = resolved(target.as_str()).unwrap();
        let uri = OsString::from(target.as_str());
        assert_eq!(launch, Launch::Open(uri.clone()));
        assert_eq!(
            plan(Platform::MacOs, &launch),
            [vec!["/usr/bin/open".into(), uri.clone()]]
        );
        assert_eq!(
            plan(Platform::Linux, &launch),
            [vec!["xdg-open".into(), uri]]
        );
        assert!(ActivationTarget::parse("javascript:alert(1)").is_err());
        assert!(ActivationTarget::parse("https://example.org/\u{202e}hidden").is_err());
    }

    #[test]
    fn non_file_schemes_are_still_opened() {
        for uri in ["http://example.org/", "mailto:a@example.org"] {
            assert_eq!(resolved(uri), Ok(Launch::Open(uri.into())));
        }
    }

    #[test]
    fn file_links_resolve_to_local_absolute_paths() {
        let host = OsStr::new("Workstation.local");
        let path = |uri| file_path(uri, host);
        let ok = |p: &str| Ok(PathBuf::from(p));
        assert_eq!(path("file:///tmp/x/evil.sh"), ok("/tmp/x/evil.sh"));
        assert_eq!(path("FILE://localhost/tmp/a"), ok("/tmp/a"));
        // `ls --hyperlink` names the machine in the authority.
        assert_eq!(path("file://workstation.LOCAL/tmp/a"), ok("/tmp/a"));
        assert_eq!(path("file:/tmp/a"), ok("/tmp/a"));
        assert_eq!(path("file:///tmp/a%20b?q=1#frag"), ok("/tmp/a b"));
        assert_eq!(
            path("file:///tmp/%E3%81%82"),
            Ok(PathBuf::from(OsString::from_vec(vec![
                b'/', b't', b'm', b'p', b'/', 0xE3, 0x81, 0x82
            ])))
        );
        assert!(path("file://elsewhere/tmp/a").is_err());
        assert!(path("file:relative").is_err());
        // A relative path could otherwise start with `-` and become an option.
        assert!(path("file:-R").is_err());
        assert!(path("file:///tmp/a%00b").is_err());
        assert!(path("file:///tmp/%zz").is_err());
        assert!(path("file:///tmp/%4").is_err());
    }

    #[test]
    fn revealed_files_use_canonical_paths_and_their_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let script = root.join("evil.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let uri = |path: &Path| file_uri(path);
        let expected = Launch::Reveal {
            item: script.clone(),
            folder: root.clone(),
        };
        assert_eq!(resolved(&uri(&script)), Ok(expected));
        // `..` after a file must not make the file itself the folder. Linux
        // refuses the path; macOS resolves it to the directory.
        match resolved(&format!("{}/..", uri(&script))) {
            Err(_) => {}
            Ok(Launch::Reveal { item, folder }) => {
                assert_eq!(item, root);
                assert!(folder.is_dir());
            }
            Ok(launch) => panic!("unexpected {launch:?}"),
        }
        assert!(resolved(&uri(&root.join("missing"))).is_err());
        let link = root.join("link");
        std::os::unix::fs::symlink(&script, &link).unwrap();
        assert_eq!(
            resolved(&uri(&link)),
            Ok(Launch::Reveal {
                item: script,
                folder: root,
            })
        );
    }

    #[test]
    fn file_links_are_revealed_instead_of_opened() {
        let launch = Launch::Reveal {
            item: "/tmp/x/a,b'c\"[d].sh".into(),
            folder: "/tmp/x".into(),
        };
        assert_eq!(
            plan(Platform::MacOs, &launch),
            [vec![
                OsString::from("/usr/bin/open"),
                "-R".into(),
                "/tmp/x/a,b'c\"[d].sh".into()
            ]]
        );
        let linux = plan(Platform::Linux, &launch);
        assert_eq!(linux.len(), 2);
        assert_eq!(linux[0][0], "gdbus");
        assert!(linux[0].contains(&"org.freedesktop.FileManager1.ShowItems".into()));
        // Quotes, commas and brackets are encoded, so GVariant text stays one string.
        assert_eq!(
            linux[0][linux[0].len() - 2],
            "['file:///tmp/x/a%2Cb%27c%22%5Bd%5D.sh']"
        );
        assert_eq!(linux[0][linux[0].len() - 1], "''");
        assert_eq!(linux[1], [OsString::from("xdg-open"), "/tmp/x".into()]);
    }

    #[test]
    fn file_link_hints_say_the_file_is_revealed() {
        assert_eq!(
            hint(&parse("file:///tmp/a"), true),
            "Cmd+click to reveal in Finder"
        );
        assert_eq!(
            hint(&parse("file:///tmp/a"), false),
            "Ctrl+click to show in the file manager"
        );
        assert_eq!(
            hint(&parse("https://example.org/"), true),
            "Cmd+click to open"
        );
        assert_eq!(
            hint(&parse("https://example.org/"), false),
            "Ctrl+click to open"
        );
    }

    #[test]
    fn a_click_while_a_link_is_opening_is_refused_visibly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = egui::Context::default();
        let mut opener = LinkOpener::default();
        opener.request(parse("https://example.org/first"));
        opener.request(parse("https://example.org/second"));
        // Before any launcher starts, the latest click is the one queued.
        assert_eq!(
            opener.queued.as_ref().map(ActivationTarget::as_str),
            Some("https://example.org/second")
        );
        opener.queued = None;
        opener.task = Some(runtime.spawn(std::future::pending()));
        opener.request(parse("https://example.org/third"));
        assert!(opener.queued.is_none());
        assert_eq!(opener.error.as_deref(), Some(BUSY));
        *opener.result.lock().unwrap() = Some(Ok(()));
        opener.tick(&runtime, &ctx);
        assert!(opener.task.is_none());
        assert_eq!(opener.error, None);
        opener.request(parse("https://example.org/fourth"));
        assert!(opener.queued.is_some());
    }

    #[test]
    fn activation_requires_the_platform_modifier_without_alt() {
        assert!(modifier(egui::Modifiers::MAC_CMD, true));
        assert!(!modifier(egui::Modifiers::CTRL, true));
        assert!(modifier(egui::Modifiers::CTRL, false));
        assert!(!modifier(egui::Modifiers::ALT, false));
        assert!(!modifier(egui::Modifiers::NONE, false));
    }
}
