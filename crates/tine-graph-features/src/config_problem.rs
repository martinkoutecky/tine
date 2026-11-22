//! The wire form of the problem a graph's config was read with, sent with the
//! graph's metadata by the desktop app's `load_graph` and by the browser core
//! (one serializer, I-12).
use serde::Serialize;
use tine_store::ConfigState;

#[derive(Debug, Serialize)]
pub struct ConfigProblem {
    kind: &'static str,
    message: String,
}

/// The read problem of `config`, if it has one. O(1), no disk access.
pub fn config_problem(config: &ConfigState) -> Option<ConfigProblem> {
    (config.problem.as_ref()).map(|error| ConfigProblem {
        kind: "config-read",
        message: error.to_string(),
    })
}
