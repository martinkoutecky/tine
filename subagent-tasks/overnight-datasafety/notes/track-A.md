# Track A - DS#1 empty asset trash safety

## What changed

- New trash writes are typed by subdirectory under `logseq/.tine-trash/`:
  - assets: `logseq/.tine-trash/assets/`
  - pages: `logseq/.tine-trash/pages/`
  - journals: `logseq/.tine-trash/journals/`
  - sync conflicts: `logseq/.tine-trash/conflicts/`
- `empty_asset_trash` now deletes only asset-type entries:
  - all entries inside the typed `assets/` trash directory;
  - legacy flat trash entries classified as assets.
- `asset_trash_stats` now treats `count` and `bytes` as asset-only totals and also reports protected non-asset counts: `pages`, `journals`, `conflicts`, and `other`.
- The orphaned-media Settings UI now labels the button as `Empty asset trash`, confirms only asset deletion, keeps page/journal/conflict recovery files, and surfaces protected recovery counts.
- Tests that asserted flat trash placement were updated to the typed `pages/` and `conflicts/` directories.

## Legacy classification rule

Legacy flat entries are classified by the original filename after the first `__` trash-stamp separator, if present. The rule is intentionally conservative:

- Sync-conflict-looking page filenames are `conflicts`.
- `.md` and `.org` files are protected as `pages`, except default Logseq journal date stems are `journals`.
- Recognized asset/media/attachment extensions are `assets`.
- Dotfiles, `.edn` sidecars, entries with path separators, directories, extensionless files, and any unknown extension are `other` and are kept.

If classification is uncertain, the entry is non-asset and survives `empty_asset_trash`.

## Evidence

- RED before implementation: after adding `empty_asset_trash_keeps_legacy_trashed_pages`, `rtk cargo test -p tine-core empty_asset_trash_keeps_legacy_trashed_pages` failed with `left: 2 right: 1`, proving the old implementation deleted both the asset and the trashed page.
- GREEN after implementation: `rtk cargo test -p tine-core empty_asset_trash_keeps_legacy_trashed_pages` passed (`1 passed`).
- Regression suite: `rtk cargo test --workspace` passed (`313 passed, 1 ignored`).
- Frontend build: `rtk npm run build` passed.
- Formatting note: `rtk cargo fmt --check` still fails due pre-existing unrelated repo-wide formatting drift; this change did not run a repo-wide formatter.
