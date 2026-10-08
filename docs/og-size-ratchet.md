# Production file-size ratchet

`src/ogEnforcement.test.ts` checks every production `.rs`, `.ts`, `.tsx`, and
`.css` file in `crates/`, `src/`, and `src-tauri/src/` against the frozen
`2d0349368` tree (og after batch 5 and its one-time `cargo fmt --all`). Counts
are production lines: inline Rust `#[cfg(test)] mod` blocks do not count. New
files may not exceed 1,500 lines. A file that was already over that limit may
not grow. Files below it may not cross it.

Oversized files at the baseline (production lines):

| File | Lines |
|---|---:|
| `crates/tine-graph-features/src/render.rs` | 2,642 |
| `crates/tine-store/src/model.rs` | 5,449 |
| `crates/tine-store/src/query.rs` | 3,669 |
| `crates/tine-store/src/store.rs` | 3,432 |
| `crates/tine-store/src/transaction.rs` | 2,244 |
| `src-tauri/src/commands.rs` | 1,963 |
| `src/components/Block.tsx` | 3,411 |
| `src/components/PdfViewer.tsx` | 2,055 |
| `src/components/Settings.tsx` | 2,830 |
| `src/components/SheetTable.tsx` | 1,682 |
| `src/mock.ts` | 1,670 |
| `src/styles/app.css` | 9,264 |
| `src/ui.ts` | 1,533 |

The failure names PARITY-CAMPAIGN §3 "Right shape" and asks for a seam split.
