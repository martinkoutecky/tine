# Query filter shortcuts for Tine

Adds a command-palette action that appends
`tine.filter:: status != "done"` to the focused query table or board. The write is
an expected-text host effect: Tine rejects it if focus or content changed, records
normal undo, and persists through its conflict-safe save path.

Build with `cargo build --release`, then run Tine's plugin checker on this directory.
Licensed MIT. AI-primary development, reviewed and published by Martin Koutecký.
