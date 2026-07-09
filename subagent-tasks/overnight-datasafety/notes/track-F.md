# Track F Notes

## Files Changed

- `src-tauri/tauri.conf.json`: desktop/Tauri identifier changed to `page.tine.Tine`.
- `src-tauri/src/migrate_identifier.rs`: current identifier changed to `page.tine.Tine`; legacy chain now includes the old desktop ID between `dev.tine.app` and the final ID; `run_early()` is a no-op on Android; tests updated and expanded with a migration-from-old-desktop-ID case.
- `src-tauri/src/lib.rs`, `src/App.tsx`, `src/backend.ts`, `CHANGELOG.md`: explanatory text updated for the final desktop ID while documenting Android retention.
- `flatpak/page.tine.Tine.yml`, `flatpak/page.tine.Tine.desktop`, `flatpak/page.tine.Tine.metainfo.xml`: renamed from the old Flatpak filenames and contents updated to the final Flatpak ID.
- `.github/workflows/flatpak.yml`, `.github/workflows/flatpak-metadata.yml`, `.github/workflows/ios-probe.yml`: Flatpak paths and iOS bundle-id comment updated to the final ID. F-Droid release workflow references were left unchanged.

## Remaining Old-ID Audit

Command used before writing this note:

`rg --no-ignore -n "page[.]tine[.]app" -g '!node_modules/**' -g '!target/**' -g '!src-tauri/gen/android/**'`

- `subagent-tasks/overnight-datasafety/TRACK-F-appid-rename.md:1,5,11,14,16,17,23,24,25,26,29,31,32,33,36,42,45,50,55,61,65,68,71`: task specification text; intentionally preserved as the source instructions for this track.
- `CHANGELOG.md:93`: historical note that the final desktop ID was briefly preceded by the old desktop ID.
- `CHANGELOG.md:98`: Android retention note; Android intentionally stays on the old ID.
- `src-tauri/src/android_folder_picker.rs:11`: Android plugin identifier; intentionally unchanged.
- `src-tauri/src/android_media.rs:17`: Android plugin identifier; intentionally unchanged.
- `src-tauri/src/migrate_identifier.rs:2,38,286,288,294,304`: legacy desktop migration source and tests proving migration from the old desktop ID to `page.tine.Tine`.
- `src-tauri/src/migrate_identifier.rs:23,160`: Android guard comments documenting that Android keeps its applicationId and is not migrated.
- `src-tauri/src/lib.rs:155`: desktop migration chain comment includes the old desktop ID as a migration source.
- `src-tauri/src/lib.rs:162`: Android guard comment; Android intentionally keeps its applicationId and `run_early()` is a no-op.
- `src/App.tsx:401`: user-facing migration notice comment includes the old desktop ID as part of the desktop migration chain.
- `src/backend.ts:292`: backend API comment includes the old desktop ID as part of the desktop migration chain.
- `src/version-code.test.ts:4,37`: F-Droid metadata comments; Android/F-Droid recipe intentionally keeps the old ID.

No remaining old-ID occurrences exist in `flatpak/`, `src-tauri/tauri.conf.json`, or Flatpak CI workflow paths.

## Validation

- `cargo test -p tine migrate`: passed (`9 passed, 11 filtered out`).
- `cargo check -p tine`: passed.
- `desktop-file-validate flatpak/page.tine.Tine.desktop`: not run; `desktop-file-validate` is not installed (`which` exit 1, direct command exit 127), no sudo is available, and apt has no `desktop-file-utils` candidate in this environment.
- `appstreamcli validate flatpak/page.tine.Tine.metainfo.xml`: not run; `appstreamcli` is not installed (`which` exit 1, direct command exit 127), no sudo is available, and apt has no `appstream` candidate in this environment.
