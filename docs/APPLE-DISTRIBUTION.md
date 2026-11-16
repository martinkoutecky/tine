# Apple distribution

Tine's existing Linux, Windows, and Android name remains **Tine**. The iOS App
Store product and home-screen display name is **TineOutline**, using the same
bundle identifier, `page.tine.Tine`. iOS Betas use the separate TestFlight-only
app **TineOutline Beta**, with Home Screen name **Tine Beta** and amber icon.

| iOS identity | Stable | Beta |
| --- | --- | --- |
| Bundle ID | `page.tine.Tine` | `page.tine.TineBeta` |
| Share Extension ID | `page.tine.Tine.ShareExtension` | `page.tine.TineBeta.ShareExtension` |
| App Group | `group.page.tine.Tine` | `group.page.tine.TineBeta` |
| iCloud container | `iCloud.page.tine.Tine` | `iCloud.page.tine.TineBeta` |
| App Store Connect Apple ID | `6804403687` | `6832052325` |
| Home Screen / iCloud folder name | TineOutline | Tine Beta |

All values derive from `src-tauri/app-identity.json` through
`scripts/set-app-identity.mjs` and its shared front door. The Beta SKU is
`tine-beta-ios`. Never submit this Beta app for production App Store review.

This document covers the two independent Apple paths:

- the ordinary release workflow signs and notarizes the direct-download macOS
  `.app` and `.dmg` with a Developer ID Application certificate;
- the manual iOS TestFlight workflow signs an IPA with an Apple Distribution
  certificate and App Store Connect provisioning profile.

Neither workflow submits a production App Store release. TestFlight upload only
places a build in App Store Connect for processing and testing.

## Repository secrets

The GitHub repository must contain these encrypted Actions secrets:

| Secret | Contents |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded password-protected Developer ID `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full Developer ID Application signing identity |
| `APPLE_TEAM_ID` | Ten-character Apple team ID |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID |
| `APPLE_API_PRIVATE_KEY` | Complete downloaded `AuthKey_*.p8` contents |
| `IOS_CERTIFICATE` | Base64-encoded password-protected Apple Distribution `.p12` |
| `IOS_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `IOS_MOBILE_PROVISION` | Base64-encoded App Store Connect provisioning profile |
| `IOS_SHARE_EXTENSION_MOBILE_PROVISION` | Base64-encoded stable Share Extension App Store profile |
| `IOS_BETA_MOBILE_PROVISION` | Base64-encoded Beta App Store profile (`RQ5V4LK7N2.page.tine.TineBeta`) |
| `IOS_BETA_SHARE_EXTENSION_MOBILE_PROVISION` | Base64-encoded Beta extension App Store profile (`RQ5V4LK7N2.page.tine.TineBeta.ShareExtension`) |
| `APP_STORE_APPLE_ID` | Numeric App Store Connect app ID, for release tooling that needs it |

Private keys and passwords must never be committed, uploaded as workflow
artifacts, printed, or copied into issue logs. Workflows materialize the API key
only in a runner-private directory, set mode `0600`, and delete it in an
`always()` cleanup step.

## macOS direct-download release

`.github/workflows/release.yml` uses the ordinary release matrix. The macOS lane
fails if any Developer ID or App Store Connect notarization secret is missing.
Tauri signs and notarizes the universal app, then the workflow independently
checks the code signature, Developer ID authority, team ID, stapled ticket,
Gatekeeper assessment, and DMG integrity before staging the release lane.

The normal exact-SHA full-CI and release-readiness gates still apply. Do not tag
or publish merely to test credentials; use a manual release smoke on an approved
candidate after its exact commit has passed full CI.

## TestFlight

Run **Actions → iOS TestFlight → Run workflow** and choose one action:

- `build-only` (default): build and verify a signed IPA, then retain it as an
  immutable workflow artifact;
- `validate`: also ask App Store Connect to validate the IPA;
- `upload`: validate and upload it for TestFlight processing.

The workflow action named `upload` is only the transport/processing step. In
maintainer conversation, **"upload it to TestFlight" means the complete external
tester delivery below**: upload, prepare, submit, wait for approval when Apple
requires it, and publish the existing tester link. If only the transport step
is wanted, say "upload the IPA to App Store Connect only."

The workflow selects Beta for `X.Y.Z-beta.N` with `ship: experiment`, and stable
for numeric `X.Y.Z` with `ship: release`. A tag must be `v` plus the checkout's
exact version; branch refs are supported for build-only checks. An identity or
tag mismatch stops before signing. `beta_as_release` is removed so a Beta can
never fall back to the App Store identity or profiles.

Marketing versions remain numeric `X.Y.Z`, with `-beta.N` stripped for Apple.
The build number contains the GitHub Actions run number, prefixed by that
version (for example `0.9.0.321`). The current `0.9.0-beta.8` candidate therefore
starts the separate Beta app at marketing version **0.9.0**, build
**0.9.0.<GITHUB_RUN_NUMBER>**; a new app does not require a version reset.
Each new dispatch gets a new run number; retrying an existing run reuses it.
The workflow verifies the selected bundle ID, display name, versions,
encryption declaration, privacy manifest, provisioning profile, team, and code
signature before it can validate or upload anything. It also compares the
compiled icon with the selected stable or amber iOS asset catalog. Upload uses
the selected numeric Apple ID and bundle ID explicitly.

The first iOS beta deliberately keeps the Wasm plugin host and plugin catalogue
off, as required by ADR 0052. Declarative built-in and token themes remain. Do
not enable iOS plugins as part of release preparation; that decision requires
the separate Apple guideline 4.7 catalogue, reporting, age-rating, and review
work recorded by the ADR.

### iCloud Drive capability

The first useful TestFlight build supports exactly two graph roots owned by
the selected app:

- stable: `On My iPhone/iPad → TineOutline` and `iCloud Drive → TineOutline`,
  backed by `iCloud.page.tine.Tine`;
- Beta: `On My iPhone/iPad → Tine Beta` and `iCloud Drive → Tine Beta`, backed
  by `iCloud.page.tine.TineBeta`.

**First Beta launch starts on Welcome, with fresh settings and no remembered
graph.** It does not see stable's settings, local graph folders, iCloud folder
or share inbox. To test existing notes, copy a graph into a Beta folder with
Files; the copy and stable graph remain independent. Data from earlier
TestFlight builds under the stable identity stays with TineOutline and is not
automatically imported. The desktop config seed does not run on iOS.

The iCloud location is the folder picker's recommended starting point when the
device is signed in to iCloud. Tine prepares ubiquitous files before handing the
graph to the ordinary guarded storage path; conflict handling remains Concord's
job just as it is for another provider-delivered external edit. Arbitrary Files
providers are intentionally outside this first boundary: Dropbox, Google Drive,
OneDrive, Working Copy and similar roots require persisted security-scoped
bookmarks and coordination that Tine does not yet implement. State this on the
Welcome screen, when refusing an outside-container folder, and in TestFlight
beta notes; do not present it as a warning about iCloud or ordinary conflicts.

Apple Developer contains separate App IDs, App Groups and iCloud containers
for both apps. Each app profile grants only its own group/container; each
extension profile grants only its app's group. Both identities share the same
Apple Distribution certificate. Regenerate the matching profile secrets after
any capability change. The workflow installs the tracked entitlements into Tauri's
generated Xcode project and verifies that both the profile and final signature
authorize the exact container and `CloudDocuments` service.

An upload is not the end of TestFlight setup. In App Store Connect, wait for
processing, answer any export-compliance prompt, complete the app privacy and
beta-review information, select internal or external testers, and submit an
external beta for Beta App Review when Apple requires it. Production App Store
submission and release remain separate manual decisions.

The manual **iOS TestFlight administration** workflow handles the repeatable
App Store Connect work after upload. Run `inspect` first. `prepare` waits for
the selected build to finish processing, installs the English beta metadata and
“What to Test” text, creates or reuses the external `Tine iOS Public Beta`
group, associates the build, and completes the non-secret review details.
`submit` sends the build to Beta App Review once the required contact fields
are complete. After Apple reports the review as approved, `publish-link`
enables a public link capped at 1,000 testers and writes it to the workflow
summary. The optional `APPLE_REVIEW_CONTACT_PHONE` secret supplies Apple's
required private review telephone number when it is not already recorded in
App Store Connect.

Administration defaults to the identity shipped by its checkout and verifies
the numeric app ID. Use a Beta ref for Beta administration. The existing
TineOutline public link is for the stable record and must not be advertised as
the separate Beta invitation; Beta needs its own tester group/link. The App
Store Connect app name is TineOutline Beta; the IPA supplies its amber icon.

For the CLI, the end-to-end maintainer sequence is:

```bash
# Build, validate, and upload the IPA. Use the intended tag or commit ref.
gh workflow run ios-testflight.yml --ref <ref> -f action=upload

# After that run succeeds, administer Apple's full CFBundleVersion. The value
# is version-prefixed (for example 0.6.97.16), not merely the Actions run number.
gh workflow run ios-testflight-admin.yml --ref <same-ref> \
  -f action=prepare -f build_number=<full-CFBundleVersion>
gh workflow run ios-testflight-admin.yml --ref <same-ref> \
  -f action=submit -f build_number=<full-CFBundleVersion>

# Once inspect reports APPROVED, make that exact build available at the
# existing public tester link.
gh workflow run ios-testflight-admin.yml --ref <same-ref> \
  -f action=publish-link -f build_number=<full-CFBundleVersion>
```

Wait for each run and inspect the exact build between stages. Completion means
the selected build is available to testers through the public link; a green
IPA upload or a `VALID` processing state alone is not completion.

## Integration gates

Before this branch is merged:

1. `https://tine.page/privacy.html` must be deployed and readable.
2. `support@tine.page` must receive and send a test reply.
3. A manual macOS candidate must pass signing, notarization, and Gatekeeper
   verification on GitHub's macOS runner, then be opened on a real Mac.
4. Both apps' App IDs, App Groups, iCloud associations and profile secrets must
   match the selected identity (configured on 2026-11-07).
5. The TestFlight workflow must pass `build-only`, then `validate`; upload needs
   Martin's explicit choice and a real iPhone smoke test.
6. App Store Connect metadata, screenshots, privacy answers, and review notes
   remain subject to Martin's approval.
7. On a real iPhone/iPad, install stable and Beta together, confirm their names
   and icons, independent first-launch settings and graph roots, iCloud access,
   Share Extension delivery, Shortcuts and Spotlight. Linux checks cannot prove
   signing, Xcode catalog compilation, App Store Connect acceptance or device
   container isolation.
