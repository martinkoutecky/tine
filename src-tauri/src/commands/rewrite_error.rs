//! One typed window boundary for rename and file-merge refusals. Other I/O
//! failures retain their existing wire strings, including rollback failures.
use tine_graph_features::pages;

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub(crate) enum RewriteError {
    Message(String),
    Refused { refused: serde_json::Value },
}

impl From<String> for RewriteError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}
impl From<&str> for RewriteError {
    fn from(message: &str) -> Self {
        Self::Message(message.into())
    }
}
impl std::fmt::Display for RewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Refused { refused } => write!(f, "{refused}"),
        }
    }
}

impl RewriteError {
    pub(crate) fn of(error: std::io::Error) -> Self {
        use pages::merge_payload::RealizationError as M;
        let refused = if let Some(check) = pages::CheckRefused::of(&error) {
            Some(
                serde_json::json!({"kind":"checkRefused", "guarantee":check.guarantee, "file":check.file}),
            )
        } else if let Some(blocked) = pages::RenameBlocked::of(&error) {
            Some(serde_json::json!(blocked))
        } else if let Some(blocked) = pages::RewriteBlocked::of(&error) {
            Some(serde_json::json!(blocked))
        } else if let Some(merge) = pages::MergeRefused::of(&error) {
            if let Some(check) = merge.check_refused() {
                return Self::Refused {
                    refused: serde_json::json!({
                        "kind":"checkRefused", "guarantee":check.guarantee, "file":check.file
                    }),
                };
            }
            let kind = match &merge.reason {
                M::HeaderChildren => Some("headerChildren"),
                M::HiddenOccurrence => Some("hiddenOccurrence"),
                M::SurvivorIdentity => Some("survivorIdentity"),
                M::BodyLaw => Some("bodyLayout"),
                M::HeaderProseOverlap => Some("headerProseOverlap"),
                M::IndentedBody => Some("indentedBody"),
                M::DirectiveEscape => Some("directiveEscape"),
                M::MissingNativeNameRange { declaration } => {
                    return Self::Refused {
                        refused: serde_json::json!({"kind":"aliasSpelling", "file":merge.file, "alias":declaration}),
                    }
                }
                M::Trace(tine_core::text_trace::TraceError::TitleIdentity) => Some("titleIdentity"),
                _ => None,
            };
            kind.map(|kind| serde_json::json!({"kind":kind, "file":merge.file}))
        } else {
            None
        };
        match refused {
            Some(refused) => Self::Refused { refused },
            None => Self::Message(error.to_string()),
        }
    }
}
