# tine-storage release checklist

This package is released independently from Tine. Tine consumes an exact
certified tag/revision and does not rerun the complete physical-storage matrix
when that pin is unchanged.

## Freeze

- [ ] The release commit is clean and pushed.
- [ ] `Cargo.toml`, `CHANGELOG.md`, and the proposed tag agree on the version.
- [ ] `api.txt` was regenerated deliberately and every public addition,
      removal, relocation, and signature change was reviewed.
- [ ] `FORMAT_MANIFEST` and `FORMAT-COMPATIBILITY.md` agree; any changed format
      has old-data migration/rebuild evidence.
- [ ] `test-support` remains absent from ordinary production dependencies.

## Certify the exact commit

- [ ] `cargo fmt --all -- --check`.
- [ ] Linux: complete default-feature and all-feature test suites, including
      public-boundary, format, crash-cut, path/no-follow, local-journal,
      authenticated-index, SQLite, and corruption tests.
- [ ] Windows: compile every target and run the complete applicable suite,
      including locking, restart, path/reparse, journal, packed-index crash
      windows, and SQLite behavior.
- [ ] Android: compile the library for the Tine Android target with the same
      minimum API/toolchain used by Tine.
- [ ] The API golden and public-boundary fixture pass.
- [ ] The certification workflow records the exact commit, Rust toolchain,
      dependency lock hash, public API hash, format manifest, and every job URL.

## Publish

- [ ] Create the annotated `vX.Y.Z` tag only for the certified commit.
- [ ] Attach the immutable certification receipt and its provenance attestation
      to the GitHub release.
- [ ] Verify the tag resolves to the receipt commit and all required jobs are
      green.
- [ ] Update Tine's exact git pin, lockfile, pin receipt, and offline Flatpak /
      F-Droid vendoring in one change; run Tine's storage integration contract.
