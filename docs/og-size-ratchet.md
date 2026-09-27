# Production file-size ratchet

`src/ogEnforcement.test.ts` checks every production `.rs`, `.ts`, `.tsx`, and
`.css` file in `crates/`, `src/`, and `src-tauri/src/` against the frozen
`aaebfb94f` tree. New files may not exceed 1,500 lines. A file that was already
over that limit may not grow. Files below it may not cross it.

Oversized files at the baseline (physical line counts):

| File | Lines |
|---|---:|
| `crates/tine-core/src/doc.rs` | 1,580 |
| `crates/tine-graph-features/src/render.rs` | 2,874 |
| `crates/tine-store/src/model.rs` | 11,100 |
| `crates/tine-store/src/query.rs` | 5,794 |
| `crates/tine-store/src/query_plan.rs` | 2,187 |
| `crates/tine-store/src/store.rs` | 5,150 |
| `crates/tine-store/src/transaction.rs` | 2,208 |
| `src-tauri/src/backup.rs` | 1,559 |
| `src-tauri/src/commands.rs` | 2,535 |
| `src/components/Block.tsx` | 3,401 |
| `src/components/PdfViewer.tsx` | 2,055 |
| `src/components/Settings.tsx` | 2,830 |
| `src/components/SheetTable.tsx` | 1,682 |
| `src/mock.ts` | 1,670 |
| `src/styles/app.css` | 9,264 |
| `src/ui.ts` | 1,533 |

The failure names PARITY-CAMPAIGN §3 "Right shape" and asks for a seam split.
