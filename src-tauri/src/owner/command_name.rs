//! The program's name in everything it prints (GH #1011): usage lines, help,
//! parse errors and the hints that tell the user what to run next.
//!
//! One answer: the file name of `argv[0]` when it is one plain word, otherwise
//! `tine`. clap is given this name (`cli::dispatch`), so its usage lines and
//! Tine's hints cannot disagree. The Beta's Linux packages install the program
//! as `tine-beta`; a literal `tine` in a hint named a command that is absent
//! there, or the stable program.

use std::sync::OnceLock;

static COMMAND_NAME: OnceLock<String> = OnceLock::new();

/// The name this process was started under, or `tine` when it never passed
/// through the command-line entry (mobile, unit tests). O(1).
pub(crate) fn command_name() -> &'static str {
    COMMAND_NAME.get().map(String::as_str).unwrap_or("tine")
}

/// Record the name from the arguments the command line is about to parse.
/// Called once per process, before clap or a hint can print it.
#[cfg(desktop)]
pub(crate) fn set_command_name(argv0: Option<&std::ffi::OsStr>) {
    if let Some(name) = command_name_from(argv0) {
        let _ = COMMAND_NAME.set(name);
    }
}

/// The file name of `argv[0]` if it is one word that needs no quoting: ASCII
/// letters, digits, `.`, `_` and `-`, not starting with `-`. `None` (the caller keeps
/// `tine`) for anything else: a space, a quote, a name that is not UTF-8.
#[cfg(desktop)]
fn command_name_from(argv0: Option<&std::ffi::OsStr>) -> Option<String> {
    let name = std::path::Path::new(argv0?).file_name()?.to_str()?;
    let plain = !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    plain.then(|| name.to_owned())
}

#[cfg(all(test, desktop))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn the_name_is_the_started_file_name_when_it_is_one_plain_word() {
        let name = |argv0: &str| command_name_from(Some(OsStr::new(argv0)));
        assert_eq!(name("tine").as_deref(), Some("tine"));
        assert_eq!(name("/usr/bin/tine-beta").as_deref(), Some("tine-beta"));
        assert_eq!(name("./target/debug/tine").as_deref(), Some("tine"));
        // As clap always printed it on Windows.
        assert_eq!(name("tine.exe").as_deref(), Some("tine.exe"));
        assert_eq!(name("my_tine.2").as_deref(), Some("my_tine.2"));
        // Not one word that needs no quoting: the caller keeps `tine`.
        for odd in [
            "",
            "/",
            "two words",
            "/opt/t/tine's",
            "ti\"ne",
            "tine\tb",
            "-tine",
            "tin\u{e9}",
            "a;b",
            "$t",
        ] {
            assert_eq!(name(odd), None, "{odd:?}");
        }
        assert_eq!(command_name_from(None), None);
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
