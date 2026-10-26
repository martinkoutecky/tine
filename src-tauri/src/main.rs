// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // GH #719: an unwritable portable folder stops here, before anything writes.
    tine_lib::portable_preflight();
    #[cfg(desktop)]
    if let Some(code) = tine_lib::cli_dispatch() {
        std::process::exit(code);
    }
    tine_lib::run()
}
