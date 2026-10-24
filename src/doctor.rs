//! A read-only frontend probe; never starts a daemon or accesses clipboard data.
use anyhow::Result;
use eframe::egui_wgpu::wgpu;
use felis_client_core::doctor::{ClipboardProbe, FontsProbe, GpuProbe, PROBE_VERSION, ProbeReport};
use serde::Serialize;
use std::{fmt::Write as _, io::Write as _, time::Duration};

#[derive(Serialize)]
struct Report {
    #[serde(flatten)]
    probe: ProbeReport,
    issues: Vec<String>,
}

impl Report {
    fn healthy(&self) -> bool {
        self.probe.gpu.available
            && self.probe.clipboard.available
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
            if probe.clipboard.available {
                "backend initialized (contents not accessed)"
            } else {
                "unavailable"
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
    let clipboard = match arboard::Clipboard::new() {
        Ok(_) => ClipboardProbe {
            available: true,
            detail: None,
        },
        Err(error) => {
            let detail = error.to_string();
            issues.push(format!("Clipboard: {detail}"));
            ClipboardProbe {
                available: false,
                detail: Some(detail),
            }
        }
    };
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
