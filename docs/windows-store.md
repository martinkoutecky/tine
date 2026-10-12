# Windows Microsoft Store package

Tine's MSIX contains the stable desktop app for x64 and ARM64. The Microsoft
Store signs accepted submissions and manages their updates. Runtime package
identity disables Tine's own update checks, notifications and installer; About
shows “Updates come from the Microsoft Store.” Sideloaded MSIX behaves the same.
Direct GitHub installs retain their updater. No submission is made by this workflow.

Partner Center values (must match exactly):

| Field | Value |
| --- | --- |
| Identity Name | `MartinKouteck.TineOutline` |
| Publisher | `CN=5B2AC29F-AAE7-4DAB-BA53-1AD73037C7F6` |
| PublisherDisplayName | `Martin Koutecký` |
| Reserved DisplayName | `TineOutline` |
| Package Family Name | `MartinKouteck.TineOutline_tctsxhb6vyy8w` |
| Store ID | `9NCZ9VKJ1TXJ` |
| Tauri data identity | Selected by `scripts/set-app-identity.mjs release` |

## Build proof

Dispatch **Microsoft Store MSIX proof** (`.github/workflows/msix.yml`) from Actions.
The optional `ref` selects source; leaving it empty builds the dispatch branch.
The source must have a stable `X.Y.Z` version: Beta refs fail before building.
The shared identity switch selects stable Tine. Both architectures are built on
`windows-latest` with embedded frontend (`custom-protocol`), staged by
`scripts/stage-msix.mjs`, and packed together by Microsoft's winapp CLI.
The template is explicitly staged as `AppxManifest.xml`, which winapp recognizes.
The existing 50, 44 and 150 pixel assets are copied. `scripts/msix-assets.ps1`
generates scale and target-size/theme variants from `icon.png` using Windows
System.Drawing, following the current Microsoft icon requirements. WACK still
needs to validate the resulting resources.

Download `TineOutline-msix-proof`: it contains the unsigned Store `.msixbundle`,
a disposable test-signed twin, its public `.cer`, SHA-256 sums and a receipt with
source SHA, version, architectures and winapp version. The private certificate
is kept only on the ephemeral runner and is excluded from uploaded artifacts.
The Store version maps `X.Y.Z` to `X.Y.Z.0`; components above 65535 and prereleases
are refused. The shared release parser also enforces its Android component bounds.

## Sideload on a test Windows machine

In an elevated PowerShell, trust the public certificate in **Trusted People**:

```powershell
Import-Certificate -FilePath .\TineOutline-test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
```

Then, as the user who will run Tine:

```powershell
Add-AppxPackage -Path .\TineOutline-test-signed.msixbundle
```

The certificate is for this test build only. Future proof runs use new certificates;
production packages are signed by Microsoft. Test with the exact bundle recorded
in the SHA-256 sums. WebView2 Evergreen must be present; MSIX does not run the
NSIS bootstrapper. Verify a clean Windows 10 machine separately.

## Data virtualization: proposed, pending confirmation

The current proposal disables filesystem and registry write virtualization using
`desktop6` properties and `unvirtualizedResources`. Martin's confirmation and
Partner Center restricted-capability approval are pending. Change the single
`unvirtualized` constant in `scripts/stage-msix.mjs` to flip both properties and
the capability together; reconsider data safety before shipping that alternative.

With the proposed opt-out, Store and GitHub stable installs share the same
`%APPDATA%\page.tine.Tine` and `%LOCALAPPDATA%\page.tine.Tine` data. Settings,
plugins, drafts, launch checkpoints and backup zips survive Store uninstall.
Default virtualization instead redirects new AppData writes to a per-package
container that uninstall removes. Graph folders outside AppData use ordinary
user permissions. Both installs register `tine:`; Windows chooses its handler.
The GitHub installation still manages its own updates. Shared single-instance
coordination and coexistence require hands-on verification.

The manifest requests exactly `runFullTrust` and `unvirtualizedResources`.
Full-trust desktop networking needs no `internetClient` capability. It uses the
classic full-trust entry point without the Windows 10 2004 `uap10` attributes,
preserving the specified Windows 10 1903 minimum.

## Open acceptance items

- Confirm the virtualization proposal and justify the restricted capabilities in Partner Center.
- Resolve the Store documentation's “first version component cannot be 0” rule against live `0.x.y.0` precedent using a draft submission; no workaround is applied.
- Run WACK on the exact bundle on a real Windows 11 box, including manifest/assets and S-mode checks.
- Complete [issue #307 §C](https://github.com/martinkoutecky/tine/issues/307): launch, graph read/write, backup/restore, plugins, drafts, capture, protocol cold/warm activation, single-instance, coexistence, upgrade/uninstall, Syncthing/network/OneDrive folders and clean-machine WebView2.
- Execute the workflow to prove Windows x64/ARM64 compilation, packing and signing. Local Linux tests cannot establish Windows runtime behavior.

No public Guide page is added while the Store listing is unpublished.

Primary references: [package identity detection](https://learn.microsoft.com/en-us/windows/msix/detect-package-identity),
[virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization),
[winapp Tauri guide](https://github.com/microsoft/winappCli/blob/main/docs/guides/tauri.md),
[winapp commands](https://github.com/microsoft/winappCli/blob/main/docs/usage.md),
[desktop capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations),
[icon requirements](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction).
