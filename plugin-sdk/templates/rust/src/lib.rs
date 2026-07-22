use tine_plugin_sdk::{notice, Effect, Event};

fn handle(event: &Event) -> Result<Vec<Effect>, String> {
    match event.kind.as_str() {
        "command" => Ok(vec![notice("Hello from a constrained Tine plugin")]),
        _ => Ok(Vec::new()),
    }
}

tine_plugin_sdk::tine_plugin!(handle);
