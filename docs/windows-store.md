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
System.Drawing, following the current Microsoft icon requirements. WACK validated
the resulting resources (PASS, run 38310552020).

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
NSIS bootstrapper. The Store package needs Windows 11.

## Data virtualization: three folders excluded (D-20261031-MSV)

Partner Center refused the earlier blanket opt-out (`desktop6` properties that
disabled all AppData and registry write virtualization). The package now uses
Windows 11 flexible virtualization with the `unvirtualizedResources` restricted
capability, and unvirtualizes exactly three folders:

```xml
<virtualization:FileSystemWriteVirtualization>
  <virtualization:ExcludedDirectories>
    <virtualization:ExcludedDirectory>$(KnownFolder:RoamingAppData)\page.tine.Tine</virtualization:ExcludedDirectory>
    <virtualization:ExcludedDirectory>$(KnownFolder:LocalAppData)\page.tine.Tine</virtualization:ExcludedDirectory>
    <virtualization:ExcludedDirectory>$(KnownFolder:LocalAppData)\tine-owners</virtualization:ExcludedDirectory>
  </virtualization:ExcludedDirectories>
</virtualization:FileSystemWriteVirtualization>
```

| Folder | Holds |
| --- | --- |
| `%APPDATA%\page.tine.Tine` | settings, plugins, themes, drafts, backup zips |
| `%LOCALAPPDATA%\page.tine.Tine` | caches and WebView2 data |
| `%LOCALAPPDATA%\tine-owners` | the per-account owner lock the app and the `tine` command line share (0.9 and later) |

Why these stay real: Store and GitHub stable installs share them, so switching
install keeps settings and drafts; backups and unsaved drafts survive Store
uninstall; and the Store app and the command line see the same owner lock, so
one graph never gets two writers. Every other AppData write, and every registry
write, is virtualized as usual. Tine's own code writes no registry; its
dependencies only read it (`hyper-util` proxy settings; `embed-resource` at build
time). Tine writes nothing machine-wide and nothing in another app's folders.

After uninstall the three folders remain. To remove all Tine data, delete them
by hand (the paths above). Flexible virtualization needs Windows 11, so the
Store package's minimum is Windows 11 (`10.0.22000.0`); Windows 10 users install
from GitHub. `msix.yml` proves the behaviour on the installed test package:
writes from inside the package reach the three real folders, a control folder
stays in the package container, Tine's own first-run data lands in the real
folders, and the three folders keep their data after uninstall.

Change the single `unvirtualized` constant in `scripts/stage-msix.mjs` to drop
the exclusions and the capability together; that fallback (full virtualization)
loses the guarantees above and needs a new decision.

The manifest requests exactly `runFullTrust` and `unvirtualizedResources`.
Full-trust desktop networking needs no `internetClient` capability. Both
installs register `tine:`; Windows chooses its handler. The GitHub installation
still manages its own updates.

## Open acceptance items

- Resubmit with the flexible-virtualization justification (D-20261031-MSV); the first submission's blanket opt-out was refused.
- Resolve the Store documentation's “first version component cannot be 0” rule against live `0.x.y.0` precedent using a draft submission; no workaround is applied.
- WACK runs in `msix.yml` on the x64 runner against the test-signed bundle and fails the build unless the
  overall result is PASS (first PASS: run 38310552020, 0.7.1.0). The kit is not available on ARM64 Windows hosts.
- Complete [issue #307 §C](https://github.com/martinkoutecky/tine/issues/307): launch, graph read/write, backup/restore, plugins, drafts, capture, protocol cold/warm activation, single-instance, coexistence, upgrade/uninstall, Syncthing/network/OneDrive folders and clean-machine WebView2.
- Execute the workflow to prove Windows x64/ARM64 compilation, packing and signing. Local Linux tests cannot establish Windows runtime behavior.

No public Guide page is added while the Store listing is unpublished.

Primary references: [package identity detection](https://learn.microsoft.com/en-us/windows/msix/detect-package-identity),
[virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization),
[winapp Tauri guide](https://github.com/microsoft/winappCli/blob/main/docs/guides/tauri.md),
[winapp commands](https://github.com/microsoft/winappCli/blob/main/docs/usage.md),
[desktop capability declarations](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations),
[icon requirements](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction).
