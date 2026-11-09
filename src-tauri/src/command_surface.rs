//! The registered Tauri command surface, as data.
//!
//! **Question answered.** [`is_known_command`] — is this string the name of a
//! command `lib.rs` registers? The diagnostic recorder uses it so a frontend
//! IPC timing event can carry a command name without ever carrying free text
//! (I-5). The build derives the names from `generate_handler!` in lib.rs;
//! the handler list is the single registration source.

include!(concat!(env!("OUT_DIR"), "/command_names.rs"));

/// Whether  is exactly a registered command name. O(log n); pure.
pub(crate) fn is_known_command(name: &str) -> bool {
    KNOWN_COMMANDS.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every name registered in `lib.rs`, module paths and `cfg` attributes
    /// stripped, on every target (master's parser: a `#[cfg(...)]` inside the
    /// handler list contains `]`, so the list end is found by bracket depth).
    fn registered() -> BTreeSet<String> {
        let lib = include_str!("lib.rs");
        let start =
            lib.find("generate_handler![").expect("handler list") + "generate_handler![".len();
        let mut depth = 1usize;
        let mut end = start;
        for (offset, byte) in lib[start..].bytes().enumerate() {
            match byte {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        lib[start..end]
            .lines()
            .map(|line| line.trim().trim_end_matches(','))
            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("#["))
            .map(|line| line.rsplit("::").next().unwrap_or(line).trim().to_string())
            .collect()
    }

    #[test]
    fn the_known_command_list_is_exactly_the_registered_handler_list() {
        let known: BTreeSet<String> = KNOWN_COMMANDS.iter().map(|name| name.to_string()).collect();
        assert_eq!(known.len(), KNOWN_COMMANDS.len(), "duplicate command name");
        assert!(
            KNOWN_COMMANDS.windows(2).all(|pair| pair[0] < pair[1]),
            "KNOWN_COMMANDS must stay sorted so is_known_command can binary-search"
        );
        assert_eq!(
            known,
            registered(),
            "KNOWN_COMMANDS must equal lib.rs generate_handler! across every target's cfg arm (sorted); a diagnostic IPC event may name only a registered command (I-5)"
        );
    }

    #[test]
    fn free_text_is_not_a_command() {
        assert!(is_known_command("page_submit"));
        assert!(!is_known_command("My secret page"));
        assert!(!is_known_command(""));
    }
}
