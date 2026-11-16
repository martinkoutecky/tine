//! Repository-local escape from the sync conversion refusal (D-20261028-CRLF).
use super::super::git_run::{open_repo_at, serialized, Repo};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Probe {
    pub(crate) safe: bool,
    pub(crate) differing: usize,
    pub(crate) sample: Option<String>,
    pub(crate) reason: Option<String>,
}

impl Probe {
    fn blocked(reason: impl Into<String>) -> Self {
        Self {
            safe: false,
            differing: 0,
            sample: None,
            reason: Some(reason.into()),
        }
    }
}

/// Read and hash each tracked regular file once, without Git's filters. Only
/// called for an autocrlf refusal, never on the successful sync path. Advice
/// only, so it takes no lock: a scan of a big graph must not hold index.lock
/// against the user's own git (or block an autocommit) on every status read.
/// Clicking repeats the probe under the locks (`disable`).
pub(crate) fn probe(root: &Path) -> Probe {
    repository(root)
        .and_then(|repo| inspect(&repo))
        .unwrap_or_else(Probe::blocked)
}

/// Same process and index locks as an autocommit, across both check and change.
/// libgit2 locks/replaces the local config atomically. No graph/index/ref write.
pub(crate) fn disable(root: &Path) -> Result<(), String> {
    serialized(|| {
        let repo = repository(root)?;
        let _index = repo.lock_index(false).map_err(|e| e.message())?;
        let result = inspect(&repo)?;
        if !result.safe {
            return Err(format!("{} tracked files differ or cannot be read, for example {}; commit your changes and follow the Guide before retrying",
                result.differing, result.sample.as_deref().unwrap_or("unknown path")));
        }
        // Local means the common repository's config, also in linked worktrees.
        // Check both libgit2's destination and its lock before any config write.
        for target in ["config", "config.lock"] {
            repo.guard
                .check_path(&repo.repo.commondir().join(target))
                .map_err(|e| e.to_string())?;
        }
        let mut local = repo
            .repo
            .config()
            .map_err(git_message)?
            .open_level(git2::ConfigLevel::Local)
            .map_err(git_message)?;
        // An included local key cannot be replaced with set_bool. Appending
        // a final override writes only the local backend file, never the include.
        local
            .set_bool("core.autocrlf", false)
            .or_else(|_| local.set_multivar("core.autocrlf", "^$", "false"))
            .map_err(git_message)
    })
}

fn git_message(error: git2::Error) -> String {
    error.message().to_owned()
}

fn repository(root: &Path) -> Result<Repo, String> {
    let repo = open_repo_at(root).map_err(|e| e.message)?;
    if !repo.prefix.as_os_str().is_empty() {
        return Err("sync requires the graph to be the repository work tree root".into());
    }
    super::super::git_autocommit::check_config(&repo).map_err(|e| e.message())?;
    Ok(repo)
}

fn inspect(repo: &Repo) -> Result<Probe, String> {
    // Report an attribute that would still block sync even without autocrlf.
    super::check_paths(repo).map_err(|e| match e {
        super::Outcome::Refused(reason) => reason,
        other => format!("{other:?}"),
    })?;
    let config = repo.repo.config().map_err(git_message)?;
    if config.get_entry("core.autocrlf").is_ok_and(|e| {
        matches!(
            e.level(),
            git2::ConfigLevel::Worktree | git2::ConfigLevel::App
        )
    }) {
        return Err("core.autocrlf is overridden above the repository-local config; turn it off there first".into());
    }
    if !(config.get_bool("core.autocrlf").unwrap_or(false)
        || config
            .get_string("core.autocrlf")
            .is_ok_and(|v| v.eq_ignore_ascii_case("input")))
    {
        return Err("core.autocrlf is not enabled; check the current sync refusal".into());
    }
    let root = repo.repo.workdir().ok_or("repository has no work tree")?;
    let binding = tine_store::BoundDirectory::new(root).map_err(|e| e.to_string())?;
    let index = repo.repo.index().map_err(git_message)?;
    let mut result = Probe {
        safe: true,
        differing: 0,
        sample: None,
        reason: None,
    };
    for entry in index.iter() {
        let path =
            std::str::from_utf8(&entry.path).map_err(|_| "sync requires UTF-8 repository paths")?;
        let absolute = root.join(path);
        let equal = matches!(entry.mode, 0o100644 | 0o100755)
            && binding.check_path(&absolute).is_ok()
            && std::fs::symlink_metadata(&absolute).is_ok_and(|m| m.is_file())
            && git2::Oid::hash_file(git2::ObjectType::Blob, &absolute)
                .is_ok_and(|oid| oid == entry.id);
        if !equal {
            result.differing += 1;
            result.sample.get_or_insert_with(|| path.to_owned());
        }
    }
    result.safe = result.differing == 0;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.precomposeunicode", true)
            .unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/a.md"), "- original\n").unwrap();
        fs::write(dir.path().join("pages/b.md"), "- second\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let me = Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &me, &me, "initial", &tree, &[])
            .unwrap();
        drop(tree);
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", true)
            .unwrap();
        (dir, repo)
    }

    #[test]
    fn crlf_fix_overrides_inherited_true_and_input_only_locally_without_file_changes() {
        for value in ["true", "input"] {
            let (dir, repo) = fixture();
            let inherited = dir.path().join("inherited.gitconfig");
            fs::write(&inherited, format!("[core]\n\tautocrlf = {value}\n")).unwrap();
            let mut config = repo.config().unwrap();
            config.remove("core.autocrlf").unwrap();
            config
                .set_str("include.path", inherited.to_str().unwrap())
                .unwrap();
            let before = fs::read(&inherited).unwrap();
            let index_before = fs::read(repo.path().join("index")).unwrap();
            let tip_before = repo.head().unwrap().target();
            assert!(probe(dir.path()).safe, "{value}");
            disable(dir.path()).unwrap();
            assert!(!repo.config().unwrap().get_bool("core.autocrlf").unwrap());
            assert_eq!(fs::read(&inherited).unwrap(), before);
            assert_eq!(fs::read(repo.path().join("index")).unwrap(), index_before);
            assert_eq!(repo.head().unwrap().target(), tip_before);
            assert_eq!(
                fs::read(dir.path().join("pages/a.md")).unwrap(),
                b"- original\n"
            );
        }
    }

    #[test]
    fn crlf_probe_counts_raw_mismatches_missing_and_nonregular_entries() {
        let (dir, repo) = fixture();
        fs::write(dir.path().join("pages/a.md"), "- original\r\n").unwrap();
        fs::remove_file(dir.path().join("pages/b.md")).unwrap();
        let got = probe(dir.path());
        assert!(!got.safe);
        assert_eq!(got.differing, 2);
        assert_eq!(got.sample.as_deref(), Some("pages/a.md"));
        assert!(disable(dir.path()).is_err());
        assert!(repo.config().unwrap().get_bool("core.autocrlf").unwrap());
        fs::write(dir.path().join("pages/a.md"), "- original\n").unwrap();
        fs::write(dir.path().join("pages/b.md"), "- second\n").unwrap();
        let mut index = repo.index().unwrap();
        let mut entry = index.get_path(Path::new("pages/a.md"), 0).unwrap();
        entry.mode = 0o160000;
        index.add(&entry).unwrap();
        index.write().unwrap();
        assert_eq!(probe(dir.path()).differing, 1, "a gitlink is never safe");
        fs::remove_file(dir.path().join("pages/b.md")).unwrap();
        fs::create_dir(dir.path().join("pages/b.md")).unwrap();
        assert_eq!(
            probe(dir.path()).differing,
            2,
            "unreadable as regular bytes"
        );
    }

    #[test]
    fn crlf_fix_refuses_remaining_attributes_and_disabled_conversion() {
        for rule in [
            "text",
            "eol=lf",
            "crlf",
            "filter=lfs",
            "ident",
            "working-tree-encoding=UTF-16",
        ] {
            let (dir, repo) = fixture();
            fs::write(
                dir.path().join(".gitattributes"),
                format!("pages/*.md {rule}\n"),
            )
            .unwrap();
            let got = probe(dir.path());
            assert!(!got.safe);
            assert!(got.reason.as_deref().unwrap().contains("pages/"), "{got:?}");
            let before = fs::read(repo.path().join("config")).unwrap();
            assert!(disable(dir.path()).is_err());
            assert_eq!(fs::read(repo.path().join("config")).unwrap(), before);
        }
        let (dir, repo) = fixture();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        assert!(!probe(dir.path()).safe);
        assert!(disable(dir.path()).is_err());
    }

    #[test]
    fn crlf_click_rechecks_bytes_and_respects_index_lock() {
        let (dir, repo) = fixture();
        assert!(probe(dir.path()).safe);
        fs::write(dir.path().join("pages/a.md"), "- edited after the notice\n").unwrap();
        let before = fs::read(repo.path().join("config")).unwrap();
        assert!(disable(dir.path()).is_err());
        assert_eq!(fs::read(repo.path().join("config")).unwrap(), before);
        fs::write(dir.path().join("pages/a.md"), "- original\n").unwrap();
        fs::write(repo.path().join("index.lock"), "other git command").unwrap();
        let error = disable(dir.path()).unwrap_err();
        assert!(error.contains("index.lock"), "{error}");
        assert_eq!(fs::read(repo.path().join("config")).unwrap(), before);
    }

    /// Writes 41 MB; run on demand (`--ignored`) to re-measure.
    #[test]
    #[ignore]
    fn crlf_probe_cost_on_ten_thousand_files() {
        let (dir, repo) = fixture();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", false)
            .unwrap();
        let mut index = repo.index().unwrap();
        for n in 2..10_000 {
            let name = format!("pages/{n}.md");
            fs::write(dir.path().join(&name), "- synthetic page\n".repeat(256)).unwrap();
            index.add_path(Path::new(&name)).unwrap();
        }
        index.write().unwrap();
        repo.config()
            .unwrap()
            .set_bool("core.autocrlf", true)
            .unwrap();
        let start = std::time::Instant::now();
        let got = probe(dir.path());
        eprintln!(
            "10k probe (10,000 tracked files, ~41 MB): {:?}",
            start.elapsed()
        );
        assert!(got.safe, "{got:?}");
    }
}
