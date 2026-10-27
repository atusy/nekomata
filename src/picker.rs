use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use felis_client_core::{Offer, connect};
use felis_protocol::messages::SessionInfo;

type Roster = Arc<Mutex<Option<Result<Vec<SessionInfo>, String>>>>;

#[derive(Default)]
pub struct SessionPicker {
    pub visible: bool,
    query: String,
    roster: Roster,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl SessionPicker {
    pub fn open(&mut self, runtime: &tokio::runtime::Runtime, ctx: &egui::Context) {
        self.visible = true;
        self.refresh(runtime, ctx);
    }

    fn refresh(&mut self, runtime: &tokio::runtime::Runtime, ctx: &egui::Context) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        // Each refresh owns its result slot, so a late old reply cannot replace it.
        self.roster = Arc::default();
        let roster = self.roster.clone();
        let ctx = ctx.clone();
        self.task = Some(runtime.spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let socket = felis_client_core::local_socket::resolve_local_socket(None)
                    .map_err(|err| err.to_string())?;
                let mut conn = connect(socket, Offer::ops())
                    .await
                    .map_err(|err| err.to_string())?;
                conn.list_sessions().await.map_err(|err| err.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("Timed out loading felis sessions".into()));
            *roster.lock().unwrap() = Some(result);
            ctx.request_repaint();
        }));
    }

    pub fn show(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        ctx: &egui::Context,
    ) -> Option<String> {
        if !self.visible {
            return None;
        }
        let mut open = true;
        let mut refresh = false;
        let mut selected = None;
        egui::Window::new("Attach a felis session")
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Session ID");
                    ui.text_edit_singleline(&mut self.query);
                    if ui.button("Attach").clicked() && !self.query.trim().is_empty() {
                        selected = Some(self.query.trim().to_owned());
                    }
                });
                refresh = ui.button("Refresh").clicked();
                ui.separator();
                let roster = self.roster.lock().unwrap();
                match &*roster {
                    None => {
                        ui.label("Loading sessions…");
                    }
                    Some(Err(err)) => {
                        ui.colored_label(crate::theme::ERROR_FG, err);
                    }
                    Some(Ok(sessions)) if sessions.is_empty() => {
                        ui.label("No felis sessions. Run nekomata to start one.");
                    }
                    Some(Ok(sessions)) => {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for session in sessions {
                                    let id = format!("{:032x}", session.id);
                                    let label = format!(
                                        "{}  ·  {}  ·  {}×{}",
                                        &id[..8],
                                        session.title.as_deref().unwrap_or("Terminal"),
                                        session.dims.cols,
                                        session.dims.rows
                                    );
                                    if ui
                                        .selectable_label(false, label)
                                        .on_hover_text(&id)
                                        .clicked()
                                    {
                                        selected = Some(id);
                                    }
                                }
                            });
                    }
                }
            });
        self.visible = open && selected.is_none();
        if refresh {
            self.refresh(runtime, ctx);
        }
        if selected.is_some() {
            self.query.clear();
        }
        selected
    }
}

impl Drop for SessionPicker {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
