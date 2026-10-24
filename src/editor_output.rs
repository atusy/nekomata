//! Local editor sessions own their staged output beyond the GUI's lifetime.
use crate::session::Target;
use anyhow::{Context, Result, bail};
use felis_client_core::{
    Offer,
    pipe::{Origin, StagedRegion},
};
use felis_protocol::messages::{GridDims, RegionPosition};
use std::path::{Path, PathBuf};

const STAGING_PREFIX: &str = "nekomata-output-";
const STAGING_FILE: &str = "output.txt";

fn staging_directory() -> std::io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(STAGING_PREFIX);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

pub struct EditorSession {
    pub id: u128,
    pub target: Target,
}

fn editor_argv(visual: Option<String>, editor: Option<String>) -> Result<Vec<String>> {
    let value = visual
        .filter(|s| !s.trim().is_empty())
        .or_else(|| editor.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".into());
    let argv = shell_words::split(&value).context("invalid VISUAL/EDITOR quoting")?;
    if argv.first().is_none_or(String::is_empty) {
        bail!("VISUAL/EDITOR must name an executable");
    }
    Ok(argv)
}

pub async fn open(
    data: Vec<u8>,
    position: Option<RegionPosition>,
    origin: Origin,
    dims: GridDims,
    target: Target,
) -> Result<EditorSession> {
    let argv = editor_argv(std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())?;
    let socket = match target {
        Target::Local(socket) => socket,
        Target::Ssh { .. } => felis_client_core::local_socket::resolve_local_socket(None)?,
    };
    let mut connection = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        felis_client_core::connect_or_spawn_daemon(&socket, Offer::ops()),
    )
    .await
    .context("local editor connection timed out")??;
    // A private, non-pid directory is intentional: felis's default staging
    // sweeper reclaims a GUI pid's files after it exits, while this editor's
    // persistent shell may still need the file.
    let dir = staging_directory()?;
    let file = dir.path().join(STAGING_FILE);
    felis_client_core::pipe::write_region_file(&data, Some(file.clone()))?;
    let executable = std::env::current_exe()?;
    let mut wrapper = vec![
        executable
            .to_str()
            .context("nekomata executable path is not UTF-8")?
            .to_owned(),
        "__edit-output".into(),
        file.to_str()
            .context("editor staging path is not UTF-8")?
            .to_owned(),
        "--".into(),
    ];
    wrapper.extend(argv);
    let prepared = felis_client_core::pipe::prepare_spawn(None, &wrapper, dims, position, &origin)?;
    // Hand ownership to the persistent helper before Create. A lost Create
    // reply is ambiguous: deleting the file could break a helper already
    // running. Such a failure can leave this one private directory behind.
    let _retained = dir.keep();
    let info = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        connection.create_with(prepared.args),
    )
    .await
    .context("create editor session timed out")?
    .context("create local editor session")?;
    Ok(EditorSession {
        id: info.id,
        target: Target::Local(socket),
    })
}

fn validate_staging_file(file: &Path) -> Result<PathBuf> {
    let parent = file.parent().context("missing staging directory")?;
    if file.file_name().and_then(|name| name.to_str()) != Some(STAGING_FILE)
        || !parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(STAGING_PREFIX))
        || parent
            .parent()
            .context("missing staging root")?
            .canonicalize()?
            != std::env::temp_dir().canonicalize()?
    {
        bail!("refusing a path outside editor staging");
    }
    let directory = std::fs::symlink_metadata(parent)?;
    let metadata = std::fs::symlink_metadata(file)?;
    if !directory.is_dir() || !metadata.is_file() {
        bail!("editor staging must not contain symlinks");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if directory.permissions().mode() & 0o077 != 0 {
            bail!("editor staging directory is not private");
        }
    }
    Ok(parent.to_owned())
}

pub fn run(file: &Path, argv: &[String]) -> Result<i32> {
    let directory = validate_staging_file(file)?;
    let (program, arguments) = argv.split_first().context("missing editor executable")?;
    if program.is_empty() {
        bail!("missing editor executable");
    }
    let guard = StagedRegion::adopt(file.to_owned());
    let result = std::process::Command::new(program)
        .args(arguments)
        .arg(file)
        .status();
    drop(guard);
    // Remove only our generated empty directory, never a recursive subtree.
    let _ = std::fs::remove_dir(directory);
    let status = result.context("start output editor")?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_arguments_preserve_quotes_without_shell_expansion() {
        assert_eq!(
            editor_argv(
                Some("editor --name 'two words' '$HOME'".into()),
                Some("other".into())
            )
            .unwrap(),
            ["editor", "--name", "two words", "$HOME"]
        );
        assert_eq!(editor_argv(None, None).unwrap(), ["vi"]);
        assert!(editor_argv(Some("'broken".into()), None).is_err());
    }
    #[test]
    fn helper_refuses_unrelated_files_and_symlinks() {
        let unrelated = tempfile::tempdir().unwrap();
        let file = unrelated.path().join(STAGING_FILE);
        std::fs::write(&file, "keep").unwrap();
        assert!(run(&file, &["true".into()]).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
        #[cfg(unix)]
        {
            let stage = tempfile::Builder::new()
                .prefix(STAGING_PREFIX)
                .tempdir()
                .unwrap();
            let linked = stage.path().join(STAGING_FILE);
            std::os::unix::fs::symlink(&file, &linked).unwrap();
            assert!(run(&linked, &["true".into()]).is_err());
            assert!(file.exists());
        }
    }
    #[test]
    fn helper_retains_file_until_child_exit_and_propagates_status() {
        let dir = staging_directory().unwrap();
        let file = dir.path().join(STAGING_FILE);
        std::fs::write(&file, "payload").unwrap();
        let path = dir.keep();
        let code = run(
            &file,
            &[
                "/bin/sh".into(),
                "-c".into(),
                "test \"$(cat \"$1\")\" = payload || exit 99; exit 7".into(),
                "test-editor".into(),
            ],
        )
        .unwrap();
        assert_eq!(code, 7);
        assert!(!file.exists());
        assert!(!path.exists());
    }
}
