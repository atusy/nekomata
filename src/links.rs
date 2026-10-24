//! Validated OSC 8 targets opened outside the UI thread.
use eframe::egui;
use felis_client_core::ActivationTarget;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

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

fn command(target: &ActivationTarget) -> Result<tokio::process::Command, String> {
    let program = if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return Err("Opening links is not supported on this platform".into());
    };
    let mut command = tokio::process::Command::new(program);
    command.arg(target.as_str()).kill_on_drop(true);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(command)
}

async fn open(target: ActivationTarget) -> Result<(), String> {
    match tokio::time::timeout(Duration::from_secs(10), command(&target)?.status()).await {
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
    fn opener_preserves_the_validated_uri_as_one_literal_argument() {
        let target =
            ActivationTarget::parse("https://example.org/?q=$(echo%20x)&a='hello'").unwrap();
        let command = command(&target).unwrap();
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args, [std::ffi::OsStr::new(target.as_str())]);
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
