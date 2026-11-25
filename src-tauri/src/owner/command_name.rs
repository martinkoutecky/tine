//! The command a printed hint tells the user to run (GH #1011).
//!
//! A hint names the program as this process was started: the file name of
//! `argv[0]`, the rule clap's usage lines already follow. The Beta's Linux
//! packages install the program as `tine-beta`, so a literal `tine` in a hint
//! named a command that is absent there, or the stable program.

use std::sync::OnceLock;

static COMMAND_NAME: OnceLock<String> = OnceLock::new();

/// The name this process was started under, or `tine` when it never passed
/// through the command-line entry (mobile, unit tests). O(1).
pub(crate) fn command_name() -> &'static str {
    COMMAND_NAME.get().map(String::as_str).unwrap_or("tine")
}

/// Record the name from the arguments the command line is about to parse.
/// Called once per process, before any hint can be printed.
#[cfg(desktop)]
pub(crate) fn set_command_name(argv0: Option<&std::ffi::OsStr>) {
    if let Some(name) = command_name_from(argv0) {
        let _ = COMMAND_NAME.set(name);
    }
}

/// The file name of `argv[0]`; on Windows without its `.exe`, so hints read
/// there as they always did. `None` (the caller keeps `tine`) for an absent,
/// empty or non-UTF-8 name.
#[cfg(desktop)]
fn command_name_from(argv0: Option<&std::ffi::OsStr>) -> Option<String> {
    let name = std::path::Path::new(argv0?).file_name()?.to_str()?;
    #[cfg(windows)]
    let name = match name.len().checked_sub(4) {
        Some(stem) if name.is_char_boundary(stem) && name[stem..].eq_ignore_ascii_case(".exe") => {
            &name[..stem]
        }
        _ => name,
    };
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn a_hint_names_the_file_name_the_process_was_started_under() {
        let name = |argv0: &str| command_name_from(Some(OsStr::new(argv0)));
        assert_eq!(name("tine").as_deref(), Some("tine"));
        assert_eq!(name("/usr/bin/tine-beta").as_deref(), Some("tine-beta"));
        assert_eq!(name("./target/debug/tine").as_deref(), Some("tine"));
        assert_eq!(name(""), None);
        assert_eq!(name("/"), None);
        assert_eq!(command_name_from(None), None);
        #[cfg(windows)]
        {
            assert_eq!(
                name(r"C:\Program Files\Tine\tine.exe").as_deref(),
                Some("tine")
            );
            assert_eq!(name("Tine-Beta.EXE").as_deref(), Some("Tine-Beta"));
            assert_eq!(name(".exe"), None);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                command_name_from(Some(OsStr::from_bytes(b"/usr/bin/t\xffne"))),
                None
            );
        }
    }

    #[test]
    fn a_process_that_never_parsed_a_command_line_says_tine() {
        // Unit tests never pass through `cli::dispatch`.
        assert_eq!(command_name(), "tine");
    }
}
