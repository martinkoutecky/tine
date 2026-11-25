//! Which Git attributes convert a path between its blob and the work tree, and
//! whether any attribute file can apply at all. Never execute filters.

use std::path::Path;

use git2::{ErrorCode, Repository};

/// Raw-content eligibility shared by runs and autocommits. Never execute
/// filters: their working bytes may be plaintext that must not enter Git.
pub(in crate::owner) fn raw_content_conversion(
    repo: &Repository,
    path: &Path,
) -> Result<Option<&'static str>, git2::Error> {
    first_set(repo, path, ["filter", "ident", "working-tree-encoding"])
}

/// The attribute that makes a checkout of `path` differ from its blob: a raw
/// content conversion ([`raw_content_conversion`]) or a line-ending one
/// (`text`, `eol`, `crlf`). Sync and pull refuse such a path (ADR 0078 §1, G5).
pub(in crate::owner) fn checkout_conversion(
    repo: &Repository,
    path: &Path,
) -> Result<Option<&'static str>, git2::Error> {
    match raw_content_conversion(repo, path)? {
        Some(attr) => Ok(Some(attr)),
        None => first_set(repo, path, ["text", "eol", "crlf"]),
    }
}

/// Whether an attribute file could set anything for one of `paths`
/// (repository-relative, `/`-separated): `false` proves every lookup for them
/// answers unspecified. These are the files libgit2 reads for a
/// `FILE_THEN_INDEX` lookup (`collect_attr_files`): [`shared_attribute_files`],
/// and a `.gitattributes` in the index or in the work-tree folder of a path or
/// of its parents (a folder's rules match only paths under it). Costs one pass
/// over the names and one `lstat` per distinct folder, never one per file;
/// any doubt answers `true`.
pub(in crate::owner) fn attribute_files_apply<'a>(
    repo: &Repository,
    paths: impl IntoIterator<Item = &'a str>,
) -> bool {
    let Some(workdir) = repo.workdir() else {
        return true;
    };
    if shared_attribute_files(repo) {
        return true;
    }
    let mut folders = std::collections::HashSet::new();
    for path in paths {
        let (mut folder, name) = path.rsplit_once('/').unwrap_or(("", path));
        if name.eq_ignore_ascii_case(".gitattributes") {
            return true;
        }
        while folders.insert(folder) && !folder.is_empty() {
            folder = folder.rfind('/').map_or("", |slash| &folder[..slash]);
        }
    }
    folders.into_iter().any(|folder| {
        #[cfg(test)]
        attr_counts::PROBES.with(|n| n.set(n.get() + 1));
        present(&workdir.join(folder).join(".gitattributes"))
    })
}

/// Whether an attribute file outside the work tree applies to every path:
/// `info/attributes`, `core.attributesFile` (else the XDG `git/attributes`),
/// or the system `gitattributes`. Any doubt answers `true`.
pub(in crate::owner) fn shared_attribute_files(repo: &Repository) -> bool {
    match repo
        .config()
        .and_then(|c| c.get_entry("core.attributesFile").map(drop))
    {
        Err(error) if error.code() == ErrorCode::NotFound => {}
        _ => return true,
    }
    if present(&repo.commondir().join("info/attributes")) {
        return true;
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    [
        (git2::ConfigLevel::System, "gitattributes"),
        (git2::ConfigLevel::XDG, "attributes"),
    ]
    .into_iter()
    .any(|(level, name)| {
        // SAFETY: Tine never sets a libgit2 search path, so no write races this read.
        let dirs = unsafe { git2::opts::get_search_path(level) }.map(|d| d.into_string());
        let Ok(Ok(dirs)) = dirs else {
            return true;
        };
        // libgit2 lets `\` escape a separator; std's split does not.
        dirs.contains(&format!("\\{separator}"))
            || std::env::split_paths(&dirs)
                .any(|dir| !dir.as_os_str().is_empty() && dir.join(name).exists())
    })
}

/// Whether something is at `path`, or could be: only "no such file" and "a
/// file where a folder should be" answer no.
fn present(path: &Path) -> bool {
    !matches!(std::fs::symlink_metadata(path), Err(error) if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    ))
}

fn first_set(
    repo: &Repository,
    path: &Path,
    attrs: [&'static str; 3],
) -> Result<Option<&'static str>, git2::Error> {
    for attr in attrs {
        #[cfg(test)]
        attr_counts::LOOKUPS.with(|n| n.set(n.get() + 1));
        let value = repo.get_attr(path, attr, git2::AttrCheckFlags::FILE_THEN_INDEX)?;
        if !matches!(
            git2::AttrValue::from_string(value),
            git2::AttrValue::Unspecified | git2::AttrValue::False
        ) {
            return Ok(Some(attr));
        }
    }
    Ok(None)
}

/// This thread's Git attribute lookups, and work-tree `.gitattributes` probes
/// made instead of them.
#[cfg(test)]
pub(crate) mod attr_counts {
    use std::cell::Cell;
    thread_local! {
        pub(crate) static LOOKUPS: Cell<usize> = const { Cell::new(0) };
        pub(crate) static PROBES: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn take() -> (usize, usize) {
        (
            LOOKUPS.with(|n| n.replace(0)),
            PROBES.with(|n| n.replace(0)),
        )
    }
}
