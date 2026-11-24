//! The config read of a graph (`logseq/config.edn`): what a read found
//! ([`ConfigFound`], one classification for an open, a memory replace, a live
//! reload and the watcher's observation) and what it changes ([`config_outcome`],
//! the one state rule; `docs/contracts/config-live-reload.md` §2).
use super::{file_source, read_parse_input_in, FileSource, ParseInputTooLarge};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tine_core::config::Config;

/// A config read: the effective config, its read problem and its text.
pub(crate) type ConfigRead = (Config, Option<crate::IoError>, Option<Arc<str>>);

/// What a config read of `root`'s [`file_source::CONFIG_FILE`] through a
/// source found: its text (or, for the watcher, its stamp), no file there, or
/// an error. The one classification a graph open, a memory replace and a live
/// reload read the config by, and the watcher observes it by
/// ([`config_found`]).
pub(crate) enum ConfigFound<T> {
    Text(T),
    Absent,
    Unreadable(io::Error),
}

/// `read` (a read, or an observation, of the config file under `root`)
/// classified by Tine's names ([`FileSource::read_error`]): a `NotFound`
/// under a file is unreadable, not absent. Free unless the read failed with
/// `NotFound`; then one stat per missing ancestor.
pub(crate) fn config_found<T>(
    source: &FileSource,
    root: &Path,
    read: io::Result<T>,
) -> ConfigFound<T> {
    match read {
        Ok(found) => ConfigFound::Text(found),
        Err(error) => match source.read_error(root, file_source::CONFIG_FILE, error) {
            error if error.kind() == io::ErrorKind::NotFound => ConfigFound::Absent,
            error => ConfigFound::Unreadable(error),
        },
    }
}

/// The config read of the graph at `root` through `source`: one read.
pub(crate) fn find_config<'a>(
    source: &'a FileSource,
    root: &Path,
) -> ConfigFound<std::borrow::Cow<'a, str>> {
    let path = root.join(file_source::CONFIG_FILE);
    config_found(source, root, read_parse_input_in(source, &path))
}

/// Settings a config read installs: the config and the text it came from.
pub(crate) type ConfigSettings = (Config, Option<Arc<str>>);

/// What a config read changes: the one state rule an open, a memory replace
/// and a live reload apply (`docs/contracts/config-live-reload.md` §2).
/// Returns the settings to install (`None` keeps those in force) and the
/// problem, a function of `found` alone:
///
/// - a text is parsed: new settings, no problem;
/// - a file over the read limit takes the reader's fail-closed verdict
///   ([`config_of_read_error`]) with its problem: new settings, as a parse;
/// - another read error keeps the settings, with its problem;
/// - absent keeps the settings, with no problem, except an iCloud
///   placeholder's ([`missing_config_problem`]) while no read text is in
///   force (`text_in_force`): then the defaults would be taken.
pub(crate) fn config_outcome(
    source: &FileSource,
    root: &Path,
    found: ConfigFound<std::borrow::Cow<str>>,
    text_in_force: bool,
) -> (Option<ConfigSettings>, Option<crate::IoError>) {
    match found {
        ConfigFound::Text(text) => (Some((Config::parse(&text), Some(Arc::from(&*text)))), None),
        ConfigFound::Absent if text_in_force => (None, None),
        ConfigFound::Absent => {
            let path = root.join(file_source::CONFIG_FILE);
            (None, missing_config_problem(source, &path))
        }
        ConfigFound::Unreadable(error) => {
            let (verdict, problem) = config_of_read_error(error);
            (verdict.map(|config| (config, None)), problem)
        }
    }
}

/// The config an open of the graph at `root` reads through `source`: the
/// defaults are in force ([`config_outcome`]).
pub(crate) fn read_config_in(source: &FileSource, root: &Path) -> ConfigRead {
    let (settings, problem) = config_outcome(source, root, find_config(source, root), false);
    let (config, text) = settings.unwrap_or_default();
    (config, problem, text)
}

/// Why a graph with no `logseq/config.edn` must not take the defaults, if it
/// must not. An iCloud placeholder (`logseq/.config.edn.icloud`) means the
/// file exists but is not downloaded: its settings are unknown, so the graph
/// opens read-only (`ConfigState::problem`) instead of writing under the
/// default :legacy file names. A plainly absent config.edn is no problem
/// (Logseq's defaults). Cost: one metadata call.
pub(crate) fn missing_config_problem(
    source: &FileSource,
    config_path: &Path,
) -> Option<crate::IoError> {
    source
        .lstat(&config_placeholder(config_path))
        .is_ok()
        .then(|| {
            io::Error::other(
                "config.edn is in iCloud but not downloaded yet; open the graph again once it is",
            )
            .into()
        })
}

/// Where iCloud keeps a not-yet-downloaded `config_path`
/// (`logseq/.config.edn.icloud`): the second path a config read looks up.
pub(crate) fn config_placeholder(config_path: &Path) -> PathBuf {
    config_path.with_file_name(".config.edn.icloud")
}

/// The settings a graph takes when its `config.edn` exists but could not be
/// read, and the problem it reports. Bytes over the store's parse limit are
/// over the EDN reader's limit too (both 64 MiB), so they take the reader's
/// verdict for oversized input, which fails closed; any other read error
/// leaves the settings in force (`None`). Cost: one type check.
pub(crate) fn config_of_read_error(error: io::Error) -> (Option<Config>, Option<crate::IoError>) {
    let too_large = error
        .get_ref()
        .is_some_and(|inner| inner.is::<ParseInputTooLarge>());
    (too_large.then(Config::over_read_limit), Some(error.into()))
}
