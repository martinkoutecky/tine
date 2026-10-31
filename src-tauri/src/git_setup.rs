//! User-requested repository setup; automatic commits/sync remain off.
use crate::owner::{git_autocommit as autocommit, git_run, git_sync};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum CloneResult {
    Cloned { root: String },
    AuthRequired,
}

fn outcome(result: git_sync::Outcome) -> Result<bool, String> {
    match result {
        git_sync::Outcome::UpToDate => Ok(true),
        git_sync::Outcome::AuthRequired => Ok(false),
        git_sync::Outcome::Refused(message) | git_sync::Outcome::Network(message) => Err(message),
        _ => Err("Git setup could not finish. Try again.".into()),
    }
}

/// No graph binding is needed: this token goes through sync's private store.
#[tauri::command]
pub(crate) async fn git_setup_sign_in(
    url: String,
    username: String,
    token: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    crate::state::off_ui(move || {
        git_sync::clone::repository_name(&url)
            .map_err(|_| "Enter an HTTPS repository URL.".to_owned())?;
        let origin = git_sync::https_origin(&url).ok_or("Enter an HTTPS repository URL.")?;
        let data = crate::graph::checkpoint_app_data(&app).ok_or("Tine has no app data folder")?;
        git_sync::credentials::Tokens::new(data).sign_in(&origin, &username, &token)
    })
    .await
}

#[tauri::command]
pub(crate) async fn git_clone_graph(
    dir: String,
    url: String,
    app: tauri::AppHandle,
) -> Result<CloneResult, String> {
    crate::state::off_ui(move || {
        let name = git_sync::clone::repository_name(&url).map_err(|e| match e {
            git_sync::Outcome::Refused(m) => m,
            _ => "Enter an HTTPS repository URL.".into(),
        })?;
        let parent = Path::new(&dir)
            .canonicalize()
            .map_err(|_| "Choose an existing parent folder.")?;
        let root = parent.join(name);
        let data = crate::graph::checkpoint_app_data(&app).ok_or("Tine has no app data folder")?;
        let operation = git_sync::Operation::new(
            Arc::new(git_sync::credentials::Tokens::new(data)),
            Arc::new(AtomicBool::new(false)),
            &git_sync::TransportOptions::default(),
        )
        .map_err(|_| "Couldn't start git transport.")?;
        if !outcome(operation.clone_graph(&url, &root))? {
            return Ok(CloneResult::AuthRequired);
        }
        // A remembered policy for a folder previously removed must not enable
        // background writes in a newly cloned graph.
        crate::settings::update_settings(&app, |json| {
            autocommit::settings_into_json(json, &root, autocommit::GitSettings::default())
        })?;
        Ok(CloneResult::Cloned {
            root: root.display().to_string(),
        })
    })
    .await
}

pub(crate) fn init_graph(
    store: &tine_store::Store,
    root: &Path,
    data: &Path,
) -> Result<(), String> {
    git_run::serialized(|| {
        store.check_graph_root().map_err(|e| format!("{e:?}"))?;
        let directory = tine_store::BoundDirectory::new(root).map_err(|e| e.to_string())?;
        directory
            .check_path(&root.join(".git"))
            .map_err(|e| e.to_string())?;
        git_run::configure_libgit2();
        match git_run::discover(root) {
            Ok(_) => return Err("This graph is already in a git repository.".into()),
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.message().into()),
        }
        match std::fs::symlink_metadata(root.join(".git")) {
            Ok(_) => return Err("The graph already contains a .git entry. Move or repair it before creating a repository.".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
        let mut options = git2::RepositoryInitOptions::new();
        options
            .initial_head("main")
            .external_template(false)
            .no_reinit(true);
        let repo =
            git2::Repository::init_opts(root, &options).map_err(|e| e.message().to_owned())?;
        let metadata = tine_store::BoundDirectory::new(repo.path()).map_err(|e| e.to_string())?;
        for file in ["config", "config.lock"] {
            metadata
                .check_path(&repo.path().join(file))
                .map_err(|e| e.to_string())?;
        }
        let mut config = repo
            .config()
            .and_then(|config| config.open_level(git2::ConfigLevel::Local))
            .map_err(|e| e.message().to_owned())?;
        config
            .set_bool("core.autocrlf", false)
            .map_err(|e| e.message().to_owned())?;
        config
            .set_bool("core.precomposeunicode", true)
            .map_err(|e| e.message().to_owned())?;
        // The same Store revision check, atomic write and staging exclusions
        // used when autocommit is enabled. No page bytes are written here.
        let setup = autocommit::prepare_git_ignore(store, root, Some(data));
        if let Err((_, message)) = &setup {
            return Err(message.clone());
        }
        autocommit::report_ignore_setup(root, setup);
        autocommit::commit_locked(root, autocommit::INITIAL_MESSAGE).map_err(|e| e.message())?;
        Ok(())
    })
}

#[tauri::command]
pub(crate) async fn git_init_graph(
    remote_url: Option<String>,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<autocommit::GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let data = crate::graph::checkpoint_app_data(&app).ok_or("Tine has no app data folder")?;
        let remote_url = remote_url.filter(|url| !url.trim().is_empty());
        if let Some(url) = &remote_url {
            git_sync::clone::repository_name(url)
                .map_err(|_| "Enter an HTTPS repository URL without credentials.")?;
        }
        crate::git_settings::set_autocommit(
            &app,
            &slot,
            autocommit::Policy::Off,
            autocommit::DEFAULT_INTERVAL_MINUTES,
        )?;
        init_graph(&slot.store, &slot.root_key, &data)?;
        if let Some(url) = remote_url {
            finish_setup(&app, &slot, &url)?;
        }
        Ok(crate::git_settings::autocommit_status(&app, &slot))
    })
    .await
}

fn configure_remote(root: &Path, url: &str) -> Result<(), String> {
    git_sync::configure_initial_remote(root, url).map_err(|why| match why {
        git_sync::Outcome::Refused(message) => message,
        _ => "Couldn't configure the initial remote.".into(),
    })
}

fn finish_setup(
    app: &tauri::AppHandle,
    slot: &crate::state::GraphSlot,
    url: &str,
) -> Result<(), String> {
    slot.store
        .check_graph_root()
        .map_err(|e| format!("{e:?}"))?;
    configure_remote(&slot.root_key, url)?;
    if url.trim().is_empty() {
        return Ok(());
    }
    let data = crate::graph::checkpoint_app_data(app).ok_or("Tine has no app data folder")?;
    let op = git_sync::Operation::new(
        Arc::new(git_sync::credentials::Tokens::new(data)),
        Arc::new(AtomicBool::new(false)),
        &git_sync::TransportOptions::default(),
    )
    .map_err(|_| "Couldn't start git transport.")?;
    autocommit::report_setup(&slot.root_key, op.push_initial(&slot.root_key));
    Ok(())
}

#[tauri::command]
pub(crate) async fn git_finish_setup(
    remote_url: String,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<autocommit::GitStatus, String> {
    let slot = crate::state::slot_for_context(&state)?;
    crate::state::off_ui(move || {
        if !git_sync::initial_push_pending(&slot.root_key) {
            return Err("No first commit is awaiting repository setup.".into());
        }
        if !remote_url.trim().is_empty() {
            git_sync::clone::repository_name(&remote_url)
                .map_err(|_| "Enter an HTTPS repository URL without credentials.")?;
        }
        finish_setup(&app, &slot, &remote_url)?;
        Ok(crate::git_settings::autocommit_status(&app, &slot))
    })
    .await
}

#[cfg(test)]
#[path = "git_setup_tests.rs"]
pub(crate) mod tests;
