# UI regression testing

Tine's UI assurance is Linux-first and drives the real production-protocol
Tauri binary through WebDriver. Browser-only tests remain useful for fast logic
and render coverage, but they are not evidence for native launch, focus, caret,
window, filesystem, or WebKit behavior.

## Durable catalog

`tests/ui-regressions/catalog.json` is the inventory of historical UI bugs and
their current proof. Add or update the catalog entry when accepting a bug,
before changing production code. A GitHub bug uses its issue number; an
internally found regression gets a stable descriptive ID. Every entry names a
behavioral boundary and points to executable evidence or an explicit exemption.

`npm run check:ui-catalog` validates the schema, issue coverage, referenced
files, and exemption reasons. The release preflight fails when the catalog is
invalid.

## Suites

- `npm run e2e:linux:smoke` is a quick local check of caret navigation,
  multi-graph isolation, and Sheets.
- `npm run e2e:linux:release` runs the complete real-app Linux release catalog.
- `npm run e2e:windows:smoke` is the advisory Windows x64 launch/edit/save/reload
  check and must run on Windows.

Build the exact candidate first:

```sh
npm run build
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml
npm run e2e:linux:release
```

The runner rejects a native binary that does not embed the current hashed
frontend asset. Each scenario receives fresh driver/native/preview ports and an
isolated artifact directory. A scenario that misses its intended assertion is a
failure; logs, result JSON, suite summary JSON, and JUnit are retained under
`test-results/e2e/` locally and uploaded by GitHub Actions.

## Graduation and release use

New or materially rewritten scenarios graduate locally only after two
consecutive complete clean runs on the same production candidate. The manual
`ui-e2e` workflow proves the hosted Linux environment; Windows remains advisory
until its reliability is established. The release workflow's Linux x64 build
lane is the hard gate once the hosted suite has three consecutive clean runs.

Do not erase a failure with a retry. Keep the first evidence, diagnose the
failure as product, harness, or infrastructure, and restart the clean-run count
after any change that can affect the result.
