# Local cap-primitives 4.0.2 patch

Pristine source: crates.io cap-primitives 4.0.2 package.
Archive SHA-256: `cdadbd7c002d3a484b35243669abdae85a0ebaded5a61117169dc3400f9a7ff0`.
Upstream repository/revision are retained in Cargo.toml and .cargo_vcs_info.json;
all upstream licenses and COPYRIGHT are retained.

Only upstream source delta: src/windows/fs/get_path.rs correctly converts an
extended UNC prefix to an ordinary UNC prefix, instead of turning the handle's
absolute network path into a relative path. Unit controls cover UNC, nested
Unicode, extended local-drive, ordinary local-drive and ordinary UNC paths.

GH #533: cap directory enumeration on both UNC and mapped Windows SMB paths
failed with OS error 3 after ordinary writes/reads and retained no-follow opens
succeeded. This corrects the path obtained from the retained handle; it adds no
ambient fallback and changes no caller confinement, no-follow or save guards.
Remove this patch when an upstream release includes the equivalent correction.
