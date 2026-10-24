//! A read-only frontend probe; never starts a daemon or accesses clipboard data.
use anyhow::Result;
use eframe::egui_wgpu::wgpu;
use felis_client_core::doctor::{ClipboardProbe, FontsProbe, GpuProbe, PROBE_VERSION, ProbeReport};
use serde::Serialize;
use std::{fmt::Write as _, io::Write as _, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ClipboardStatus {
    Available,
    Unavailable,
    Unverified,
}

fn clipboard_probe(result: Result<(), String>, wayland: bool) -> (ClipboardProbe, ClipboardStatus) {
    match result {
        Ok(()) => (
            ClipboardProbe {
                available: true,
                detail: None,
            },
            ClipboardStatus::Available,
        ),
        Err(error) if wayland => (
            ClipboardProbe {
                available: false,
                detail: Some(format!(
                    "Unverified: the native Wayland clipboard requires a window; the standalone backend probe failed: {error}"
                )),
            },
            ClipboardStatus::Unverified,
        ),
        Err(error) => (
            ClipboardProbe {
                available: false,
                detail: Some(error),
            },
            ClipboardStatus::Unavailable,
        ),
    }
}

#[derive(Serialize)]
struct Report {
    #[serde(flatten)]
    probe: ProbeReport,
    issues: Vec<String>,
    clipboard_status: ClipboardStatus,
}

impl Report {
    fn healthy(&self) -> bool {
        self.probe.gpu.available
            && self.clipboard_status != ClipboardStatus::Unavailable
            && matches!(self.probe.fonts, Some(FontsProbe::Resolved { .. }))
    }

    fn human(&self) -> String {
        let probe = &self.probe;
        let mut text = format!("nekomata {}\n", probe.client_version);
        let gpu = &probe.gpu;
        let _ = writeln!(
            text,
            "GPU: {} ({}, {}) — {}",
            gpu.name.as_deref().unwrap_or("none"),
            gpu.backend.as_deref().unwrap_or("unknown"),
            gpu.device_type.as_deref().unwrap_or("unknown"),
            if gpu.available {
                "device ready"
            } else {
                "unavailable"
            }
        );
        let _ = writeln!(
            text,
            "Clipboard: {}",
            match self.clipboard_status {
                ClipboardStatus::Available => "backend initialized (contents not accessed)",
                ClipboardStatus::Unavailable => "unavailable",
                ClipboardStatus::Unverified =>
                    "unverified (native Wayland clipboard requires a window)",
            }
        );
        match &probe.fonts {
            Some(FontsProbe::Resolved {
                regular,
                bold,
                italic,
                bold_italic,
                fallbacks,
                system_fallback,
            }) => {
                let _ = writeln!(
                    text,
                    "Fonts: {regular}\n  Bold: {bold}\n  Italic: {italic}\n  Bold italic: {bold_italic}\n  Fallbacks: {}\n  System fallback: {}",
                    fallbacks.join(", "),
                    system_fallback.as_deref().unwrap_or("none")
                );
            }
            Some(FontsProbe::Failed { error }) => {
                let _ = writeln!(text, "Fonts: unavailable ({error})");
            }
            None => {}
        }
        for issue in &self.issues {
            let _ = writeln!(text, "Issue: {issue}");
        }
        text.push_str(
            "Probe only: native surface presentation and clipboard transfer are not tested.\n",
        );
        text
    }
}

pub fn run(json: bool) -> Result<bool> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut issues = Vec::new();
    let gpu = match runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(10), probe_gpu()).await })
    {
        Ok(Ok(gpu)) => gpu,
        Ok(Err(error)) => {
            issues.push(format!("GPU: {error:#}"));
            GpuProbe::unavailable()
        }
        Err(_) => {
            issues.push("GPU probe timed out".into());
            GpuProbe::unavailable()
        }
    };
    // egui-winit can use Smithay's ordinary Wayland clipboard with a window,
    // even when arboard's data-control protocol and X11 fallback are absent.
    let (clipboard, clipboard_status) = clipboard_probe(
        arboard::Clipboard::new()
            .map(|_| ())
            .map_err(|error| error.to_string()),
        cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some(),
    );
    if clipboard_status == ClipboardStatus::Unavailable {
        issues.push(format!(
            "Clipboard: {}",
            clipboard.detail.as_deref().unwrap_or_default()
        ));
    }
    let fonts = match crate::render::load_fonts(&eframe::egui::Context::default()) {
        Ok(stack) => {
            let description = stack.describe();
            let [regular, bold, italic, bold_italic] = description.primary;
            FontsProbe::Resolved {
                regular,
                bold,
                italic,
                bold_italic,
                fallbacks: description.fallbacks,
                system_fallback: description.system_fallback,
            }
        }
        Err(error) => FontsProbe::Failed {
            error: format!("{error:#}"),
        },
    };
    let report = Report {
        probe: ProbeReport {
            v: PROBE_VERSION,
            client_version: env!("CARGO_PKG_VERSION").into(),
            gpu,
            clipboard,
            fonts: Some(fonts),
        },
        issues,
        clipboard_status,
    };
    let text = if json {
        serde_json::to_string_pretty(&report)? + "\n"
    } else {
        report.human()
    };
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(report.healthy())
}

async fn probe_gpu() -> Result<GpuProbe> {
    let setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    let instance = wgpu::Instance::new(setup.instance_descriptor);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: setup.power_preference,
            ..Default::default()
        })
        .await?;
    anyhow::ensure!(
        adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::VIEW_FORMATS),
        "compatible texture views are required by the terminal renderer"
    );
    let (_device, _queue) = adapter
        .request_device(&(setup.device_descriptor)(&adapter))
        .await?;
    let info = adapter.get_info();
    Ok(GpuProbe {
        available: true,
        name: Some(info.name),
        backend: Some(format!("{:?}", info.backend).to_lowercase()),
        device_type: Some(
            match info.device_type {
                wgpu::DeviceType::Other => "other",
                wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
                wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
                wgpu::DeviceType::VirtualGpu => "virtual_gpu",
                wgpu::DeviceType::Cpu => "cpu",
            }
            .into(),
        ),
        driver: Some(info.driver),
        driver_info: Some(info.driver_info),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_data_control_does_not_diagnose_the_wayland_gui_as_broken() {
        let (probe, status) = clipboard_probe(Err("no data-control protocol".into()), true);
        assert_eq!(status, ClipboardStatus::Unverified);
        assert!(!probe.available);
        assert!(probe.detail.as_ref().unwrap().contains("requires a window"));
        assert_eq!(serde_json::to_value(status).unwrap(), "unverified");
        let report = Report {
            probe: ProbeReport {
                v: PROBE_VERSION,
                client_version: "test".into(),
                gpu: GpuProbe {
                    available: true,
                    ..GpuProbe::unavailable()
                },
                clipboard: probe,
                fonts: Some(FontsProbe::Resolved {
                    regular: "font".into(),
                    bold: "font".into(),
                    italic: "font".into(),
                    bold_italic: "font".into(),
                    fallbacks: vec![],
                    system_fallback: None,
                }),
            },
            issues: vec![],
            clipboard_status: status,
        };
        assert!(report.healthy(), "unverified must not fail the diagnosis");
        assert!(report.human().contains("Clipboard: unverified"));
        assert_eq!(
            clipboard_probe(Err("no display".into()), false).1,
            ClipboardStatus::Unavailable
        );
        assert_eq!(clipboard_probe(Ok(()), true).1, ClipboardStatus::Available);
    }

    #[test]
    fn unavailable_components_remain_visible_in_human_and_shared_json_reports() {
        let report = Report {
            probe: ProbeReport {
                v: PROBE_VERSION,
                client_version: "test".into(),
                gpu: GpuProbe::unavailable(),
                clipboard: ClipboardProbe {
                    available: false,
                    detail: Some("no display".into()),
                },
                fonts: Some(FontsProbe::Failed {
                    error: "missing font".into(),
                }),
            },
            issues: vec!["Clipboard: no display".into()],
            clipboard_status: ClipboardStatus::Unavailable,
        };
        assert!(!report.healthy());
        let human = report.human();
        assert!(human.contains("no display"));
        assert!(human.contains("missing font"));
        let shared: ProbeReport =
            serde_json::from_value(serde_json::to_value(report).unwrap()).unwrap();
        assert!(!shared.gpu.available);
        assert!(matches!(shared.fonts, Some(FontsProbe::Failed { .. })));
    }
}
