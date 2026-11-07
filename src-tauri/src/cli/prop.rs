use super::*;
use crate::owner::terminal::write_verb;
use serde_json::{json, Value};

#[derive(Debug, Subcommand)]
pub(super) enum PropAction {
    /// Set KEY to VALUE on a block, page or journal.
    Set {
        #[arg(value_name = "BLOCK_OR_KEY")]
        first: String,
        #[arg(value_name = "KEY_OR_VALUE", allow_hyphen_values = true)]
        second: String,
        #[arg(value_name = "VALUE", allow_hyphen_values = true, required_unless_present_any = ["page", "journal"], conflicts_with_all = ["page", "journal"])]
        third: Option<String>,
        #[command(flatten)]
        target: PageTarget,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Remove KEY from a block, page or journal.
    Remove {
        #[arg(value_name = "BLOCK_OR_KEY")]
        first: String,
        #[arg(value_name = "KEY", required_unless_present_any = ["page", "journal"], conflicts_with_all = ["page", "journal"])]
        second: Option<String>,
        #[command(flatten)]
        target: PageTarget,
        #[command(flatten)]
        write: WriteArgs,
    },
}

#[derive(Debug, Args)]
#[group(multiple = false)]
pub(super) struct PageTarget {
    /// Edit properties of an existing PAGE.
    #[arg(long, value_name = "PAGE")]
    page: Option<String>,
    /// Edit an existing journal: today, or --journal=DATE.
    #[arg(long, value_name = "DATE", num_args = 0..=1, require_equals = true, default_missing_value = "today")]
    journal: Option<String>,
}

impl PropAction {
    pub(super) fn dispatch_property(self) -> i32 {
        let (mut operands, target, write, set) = match self {
            Self::Set {
                first,
                second,
                third,
                target,
                write,
            } => (
                vec![Some(first), Some(second), third]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>(),
                target,
                write,
                true,
            ),
            Self::Remove {
                first,
                second,
                target,
                write,
            } => (
                vec![Some(first), second].into_iter().flatten().collect(),
                target,
                write,
                false,
            ),
        };
        let page = target.page.is_some() || target.journal.is_some();
        let value = if set {
            json!(operands.pop().unwrap())
        } else {
            Value::Null
        };
        let key = operands.pop().unwrap();
        let args = write.args(json!({"handle":operands.pop(),"page":target.page,"journal":target.journal,"key":key,"value":value}));
        write_verb(
            write.read.target(),
            if page { "page.prop" } else { "block.prop" },
            args,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gh839_property_targets_parse_with_write_flags_after_operands() {
        for target in [
            vec!["--page", "Page"],
            vec!["--journal"],
            vec!["--journal=2026-11-01"],
        ] {
            let mut args = vec!["tine", "prop", "set"];
            args.extend(target);
            args.extend([
                "tags",
                "technical",
                "--graph",
                "/scratch/graph",
                "--rev",
                "rev",
                "--json",
            ]);
            let parsed = Cli::try_parse_from(args).unwrap();
            let Some(Command::Prop {
                action: PropAction::Set { third, write, .. },
            }) = parsed.command
            else {
                panic!("not prop set")
            };
            assert_eq!(third, None);
            assert_eq!(write.rev.as_deref(), Some("rev"));
        }
        assert!(Cli::try_parse_from([
            "tine",
            "prop",
            "set",
            "Block/1~abcdef",
            "tags",
            "technical"
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["tine", "prop", "remove", "--page", "Page", "tags"]).is_ok());
        assert!(Cli::try_parse_from([
            "tine",
            "prop",
            "set",
            "--page",
            "Page",
            "--journal",
            "tags",
            "technical"
        ])
        .is_err());
    }
}
