// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(desktop)]
    if let Some(code) = tine_lib::cli_dispatch() {
        std::process::exit(code);
    }
    tine_lib::run()
}
