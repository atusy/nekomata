use super::*;
use std::os::unix::fs::PermissionsExt;

fn dormant_session(
    runtime: &tokio::runtime::Runtime,
    ctx: &egui::Context,
    id: u128,
) -> Arc<Session> {
    // A current-thread runtime that is never driven cannot contact a daemon.
    let session = Arc::new(Session::start(
        runtime,
        ctx.clone(),
        Target::Local("/unused-editor-view-test.sock".into()),
        Some(format!("{id:032x}")),
        None,
        vec![],
    ));
    {
        let mut state = session.state.lock().unwrap();
        state.connected = true;
        state.session_id = Some(id);
    }
    session
}

#[test]
fn editor_exit_restores_the_original_session_and_selection() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ctx = egui::Context::default();
    let original = dormant_session(&runtime, &ctx, 1);
    let editor = dormant_session(&runtime, &ctx, 2);
    let mut pane = Pane::from_session(original.clone());
    pane.dims = (80, 24);
    pane.selection = Some(Selection::new(GridPos { row: 2, col: 3 }));
    pane.reported_focus = Some(true);
    pane.search_open = true;
    pane.search_query = "original query".into();
    let selection = pane.selection;
    original.output.output.lock().unwrap().result = Some(Err("old output reply".into()));

    pane.show_editor(editor.clone());
    assert!(original.output.output.lock().unwrap().result.is_none());
    assert!(Arc::ptr_eq(&pane.session, &editor));
    assert!(Arc::ptr_eq(pane.persisted_session(), &original));
    assert_eq!(pane.return_to.as_ref().unwrap().reported_focus, Some(false));
    assert!(
        !pane.search_open,
        "editor input must not enter the old search field"
    );

    editor.state.lock().unwrap().exited = true;
    assert!(pane.restore_after_editor().is_none());
    assert!(Arc::ptr_eq(&pane.session, &original));
    assert!(pane.return_to.is_none());
    assert_eq!(pane.dims, (80, 24));
    assert_eq!(pane.selection, selection);
    assert_eq!(pane.reported_focus, None);
    assert!(pane.search_open);
    assert_eq!(pane.search_query, "original query");
    assert!(original.state.lock().unwrap().connected);
}

#[test]
fn reconnecting_editor_keeps_its_temporary_view_until_retry_finishes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ctx = egui::Context::default();
    let original = dormant_session(&runtime, &ctx, 1);
    let editor = dormant_session(&runtime, &ctx, 2);
    let mut pane = Pane::from_session(original.clone());
    pane.show_editor(editor.clone());
    {
        let mut state = editor.state.lock().unwrap();
        state.connected = false;
        state.reconnecting = true;
        state.error = Some("temporary transport loss".into());
    }

    assert!(pane.restore_after_editor().is_none());
    assert!(Arc::ptr_eq(&pane.session, &editor));
    assert!(Arc::ptr_eq(pane.persisted_session(), &original));
}

#[test]
fn failed_editor_attachment_returns_to_the_original_session() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ctx = egui::Context::default();
    let original = dormant_session(&runtime, &ctx, 1);
    let editor = dormant_session(&runtime, &ctx, 2);
    let mut pane = Pane::from_session(original.clone());
    pane.show_editor(editor.clone());
    {
        let mut state = editor.state.lock().unwrap();
        state.connected = false;
        state.error = Some("editor session no longer exists".into());
    }

    let error = pane
        .restore_after_editor()
        .expect("surface the attach failure");
    assert!(error.contains("editor session no longer exists"), "{error}");
    assert!(Arc::ptr_eq(&pane.session, &original));
    assert!(pane.return_to.is_none());
}

fn test_app(runtime: tokio::runtime::Runtime, ctx: &egui::Context) -> App {
    let control = control::Server::bind(&runtime, ctx.clone()).unwrap();
    App {
        workspace: None,
        workspace_id: None,
        workspace_error: None,
        last_snapshot: None,
        pending_workspace: None,
        workspaces_open: false,
        workspace_roster: None,
        runtime,
        control,
        pending_attach: BTreeMap::new(),
        pending_editors: vec![],
        output_error: None,
        panes: BTreeMap::new(),
        tabs: vec![],
        active: 0,
        next_id: 1,
        font_size: 15.0,
        picker: SessionPicker::default(),
        help: false,
        shortcuts: Shortcuts::default(),
        palette: false,
        palette_query: String::new(),
    }
}

fn wait_for(app: &mut App, ctx: &egui::Context, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let _ = crate::test_support::run(ctx, egui::RawInput::default(), |_| {});
        app.poll_command_output(ctx);
        if done(app) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "editor view did not settle"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn completed_editor(
    app: &mut App,
    pane: u64,
    origin: Arc<Session>,
    generation: u64,
    editor: u128,
    target: Target,
) -> tempfile::TempDir {
    let staging = tempfile::Builder::new()
        .prefix("nekomata-output-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let staged_file = staging.path().join("output.txt");
    std::fs::write(&staged_file, "test command output").unwrap();
    let (send, result) = tokio::sync::oneshot::channel();
    app.pending_editors.push(PendingEditor {
        pane,
        origin,
        generation,
        result,
    });
    assert!(
        send.send(Ok(crate::editor_output::EditorSession {
            id: editor,
            target,
            staged_file,
        }))
        .is_ok()
    );
    staging
}

#[test]
#[ignore = "requires an isolated felis daemon; run scripts/test-live.sh via nix develop"]
fn live_editor_returns_in_place_without_changing_layout_or_focus() {
    use felis_client_core::Offer;
    use felis_protocol::messages::SpawnArgs;

    let socket = std::env::var("NEKOMATA_TEST_SOCKET").expect("use scripts/test-live.sh");
    assert_eq!(std::env::var("FELIS_SOCKET").unwrap(), socket);
    let target = Target::Local(socket.clone().into());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let ids = runtime.block_on(async {
        let mut ops =
            felis_client_core::connect_or_spawn_daemon(std::path::Path::new(&socket), Offer::ops())
                .await
                .unwrap();
        let mut ids = vec![];
        let shell_args = vec![
            "-c".into(),
            r#"while IFS= read -r line; do eval "$line"; done"#.into(),
        ];
        let editor_args = vec!["-c".into(), "printf 'EDITOR_READY\\n'; read -r line".into()];
        for args in [
            shell_args.clone(),
            shell_args,
            editor_args.clone(),
            editor_args.clone(),
            editor_args,
        ] {
            ids.push(
                ops.spawn_session(SpawnArgs {
                    command: "/bin/sh".into(),
                    args,
                    env_base: felis_client_core::env_base::capture(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .id,
            );
        }
        ids
    });
    let ctx = egui::Context::default();
    let mut app = test_app(runtime, &ctx);
    app.add_tab(&ctx, Some(format!("{:032x}", ids[0])));
    app.add_tab(&ctx, Some(format!("{:032x}", ids[1])));
    wait_for(&mut app, &ctx, |app| {
        app.panes
            .values()
            .all(|pane| pane.session.state.lock().unwrap().connected)
    });
    app.active = 0;
    let original = app.panes[&1].session.clone();
    let generation = original.state.lock().unwrap().reconnects;
    let before = app.snapshot();
    let _editor_staging = completed_editor(
        &mut app,
        1,
        original.clone(),
        generation,
        ids[2],
        target.clone(),
    );
    app.poll_command_output(&ctx);
    assert_eq!(
        app.snapshot(),
        before,
        "temporary editor must not be persisted"
    );
    assert_eq!(app.panes.len(), 2);
    assert!(app.panes[&1].return_to.is_some());
    wait_for(&mut app, &ctx, |app| {
        app.panes[&1].session.state.lock().unwrap().connected
    });
    assert_eq!(
        app.panes[&1].session.state.lock().unwrap().session_id,
        Some(ids[2])
    );
    assert!(original.state.lock().unwrap().connected);

    let fixture = tempfile::tempdir_in(std::path::Path::new(&socket).parent().unwrap()).unwrap();
    let marker = fixture.path().join("original-still-alive");
    original.send(InputMsg::KeyBytes(
        format!(
            "printf alive > {}\r",
            shell_words::quote(marker.to_str().unwrap())
        )
        .into_bytes(),
    ));
    wait_for(&mut app, &ctx, |_| marker.exists());
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "alive");

    // A user moves elsewhere while editing: completion must not steal focus.
    app.active = 1;
    let mut expected = before;
    expected.active = 1;
    let editor = app.panes[&1].session.clone();
    editor.send(InputMsg::KeyBytes(b"\r".to_vec()));
    wait_for(&mut app, &ctx, |app| app.panes[&1].return_to.is_none());
    assert!(Arc::ptr_eq(&app.panes[&1].session, &original));
    assert_eq!(app.snapshot(), expected);
    assert!(original.state.lock().unwrap().connected);

    // A delayed completion for an obsolete connection cannot replace this pane.
    let stale_generation = completed_editor(
        &mut app,
        1,
        original.clone(),
        generation.wrapping_add(1),
        ids[3],
        target.clone(),
    );
    app.poll_command_output(&ctx);
    assert!(Arc::ptr_eq(&app.panes[&1].session, &original));
    assert_eq!(app.snapshot(), expected);
    wait_for(&mut app, &ctx, |app| {
        !stale_generation.path().exists() && app.pending_editors.is_empty()
    });
    let other = app.panes[&2].session.clone();
    let other_generation = other.state.lock().unwrap().reconnects;
    let stale_origin = completed_editor(&mut app, 1, other, other_generation, ids[4], target);
    app.poll_command_output(&ctx);
    assert!(Arc::ptr_eq(&app.panes[&1].session, &original));
    assert_eq!(app.snapshot(), expected);

    wait_for(&mut app, &ctx, |app| {
        !stale_origin.path().exists() && app.pending_editors.is_empty()
    });
    app.runtime.block_on(async {
        let mut ops = felis_client_core::connect(std::path::Path::new(&socket), Offer::ops())
            .await
            .unwrap();
        let sessions = ops.list_sessions().await.unwrap();
        assert!(
            !sessions
                .iter()
                .any(|session| session.id == ids[3] || session.id == ids[4])
        );
        for id in ids {
            let _ = ops.destroy_session(format!("{id:032x}")).await;
        }
    });
}
