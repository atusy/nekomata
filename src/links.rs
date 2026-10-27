//! Validated OSC 8 targets opened outside the UI thread.
use eframe::egui;
use felis_client_core::ActivationTarget;
use std::{
    ffi::OsString,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Bounds a launcher that never exits, so one stuck opener cannot keep the
/// pane busy forever.
const OPENER_TIMEOUT: Duration = Duration::from_secs(10);

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
    pub fn request(&mut self, target: ActivationTarget) {
        if self.queued.is_none() && self.task.is_none() {
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

/// Launchers to try in order until one succeeds. Pure so every platform's
/// argv is testable on any host.
fn plan(platform: Platform, target: &ActivationTarget) -> Vec<Step> {
    let program = match platform {
        Platform::MacOs => "/usr/bin/open",
        Platform::Linux => "xdg-open",
    };
    vec![vec![program.into(), target.as_str().into()]]
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
    let mut outcome = Ok(());
    for step in plan(platform, &target) {
        outcome = run(&step).await;
        if outcome.is_ok() {
            break;
        }
    }
    outcome
}

async fn run(step: &[OsString]) -> Result<(), String> {
    match tokio::time::timeout(OPENER_TIMEOUT, command(step).status()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!(
            "Could not open the link ({status}); check the default application for this URL type"
        )),
        Ok(Err(error)) => Err(format!("Could not start the link opener: {error}")),
        Err(_) => Err(
            "Opening the link timed out; check the default application for this URL type".into(),
        ),
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

    #[test]
    fn opener_preserves_the_validated_uri_as_one_literal_argument() {
        let target =
            ActivationTarget::parse("https://example.org/?q=$(echo%20x)&a='hello'").unwrap();
        let uri = OsString::from(target.as_str());
        assert_eq!(
            plan(Platform::MacOs, &target),
            [vec!["/usr/bin/open".into(), uri.clone()]]
        );
        assert_eq!(
            plan(Platform::Linux, &target),
            [vec!["xdg-open".into(), uri]]
        );
        assert!(ActivationTarget::parse("javascript:alert(1)").is_err());
        assert!(ActivationTarget::parse("https://example.org/\u{202e}hidden").is_err());
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
