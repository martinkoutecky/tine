use tine_plugin_sdk::{Effect, Event};

// The visible sentinel is the host-owned declarative thread-lines contribution.
// The guest intentionally performs no edits, notices, or other side effects.
fn handle(_event: &Event) -> Result<Vec<Effect>, String> {
    Ok(Vec::new())
}

tine_plugin_sdk::tine_plugin!(handle);
