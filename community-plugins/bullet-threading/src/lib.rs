use tine_plugin_sdk::{Effect, Event};

// The visual is declarative and rendered by Tine. The guest intentionally has no
// dynamic work: enabling its contribution is the whole feature.
fn handle(_event: &Event) -> Result<Vec<Effect>, String> {
    Ok(Vec::new())
}

tine_plugin_sdk::tine_plugin!(handle);
