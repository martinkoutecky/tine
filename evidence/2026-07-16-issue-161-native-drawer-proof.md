# GH #161 E2 — native Linux/WebKit drawer proof

Date: 2026-07-16

## Scope

This receipt closes the E2 Linux/WebKit observation row from the authoritative
GH #161 specification. It does not claim Android hardware-Back proof.

The retained scenario is `scripts/e2e-mobile-drawers.mjs`, registered as
`mobile-drawers` in the release-gated `linux-release` suite. The old
`coveredBy` placeholders were removed: every acceptance item below is observed
in the real Tauri/WebKit release binary.

## Candidate

- Built with the sanctioned production protocol:
  `TINE_DEPLOY_DEST=/tmp/tine-issue161-candidate ./scripts/deploy.sh`.
- Frontend build/oracle checks, release Rust build, and embedded-current-dist
  check passed.
- Candidate: `target/release/tine`.
- Candidate SHA-256:
  `bd846f1ad2474cce1b095fe9124f93a67793e03fac329a051854c5d4231b9112`.
- No product source was changed in response to the native run.

## Forced real-window flow

The first process uses only the Rust test geometry policy. The frontend then
classifies the real `window.innerWidth`; the harness does not mutate the
classifier or any sidebar signal.

- Observed viewport: exactly `390x844`; `data-mobile-drawer-mode="true"`.
- Left drawer: unchanged main rectangle; left-edge anchor; width cap leaving at
  least 44 px; one scrim/panel; hidden resizer; dialog/`aria-modal`; focus inside.
- Consuming pointer: a native pointer clicked the literal screen coordinates of
  the inert left-toolbar toggle while the right drawer was open. `elementFromPoint`
  proved the scrim was the hit target; the drawer closed, the covered toggle did
  not open the other drawer, the route remained `Drawer target`, and focus moved
  to main content.
- Left navigation: a real click on the seeded Favorite opened `Drawer target`,
  closed the drawer, and focused main content.
- Right drawer: unchanged main rectangle; right-edge anchor/cap; one modal
  panel/scrim; hidden resizer; inert background; a background focus attempt was
  contained in the drawer.
- Escape: first Escape closed only the real right-sidebar actions menu and
  focused its button; second Escape closed the drawer and restored the actual
  toolbar opener without navigation.
- Live editor: native page completion opened and consumed its first Escape;
  a synthetic `isComposing` Escape left the editor/drawer intact; the subsequent
  plain Escape closed the whole drawer without entering block selection.
- Safe edit: `Native saved [[Completion Target]] 中文` reached the page file,
  survived drawer reopen, then survived a fresh same-XDG native process.
- Exclusivity through real shortcuts: `left -> right -> left`, with exactly one
  panel and one scrim at each step.

## Unforced regular-width neighbor

A third process removed `TINE_E2E_FORCE_MOBILE_DRAWERS` and used the same
candidate and graph.

- At `960x760`, the classifier was `false`; left and right sidebars were open
  simultaneously, consumed their persisted flex widths, exposed both resizers,
  and had no scrim, inert background, dialog role, or `aria-modal` state.
- The same unforced process widened to `1600x900` for the split/PDF structural
  neighbor. This avoids an invalid 960 px assertion that tried to fit a fixed
  560 px PDF plus two panes plus both fixed-width sidebars. The right persistent
  sidebar remained fixed; two real panes and the real PDF pane remained inside
  `.drawer-workspace`, and the PDF parent contract stayed intact.

## Command and result

Final gate:

```text
rtk proxy node scripts/run-e2e.mjs linux-release --scenario=mobile-drawers
PASS mobile-drawers (14.3s)
E2E linux-release: 1 passed, 0 failed
```

Runner result: one attempt, no infrastructure retry, `14261 ms`.

Retained artifacts:

- `test-results/e2e/linux-release/mobile-drawers/proof.json`
  (SHA-256 `3ac6f8dfcad08e27dc88a5b49681764baedfd57899705defe83aa54f0a27475d`)
- `forced-left.png`
- `forced-right.png`
- `forced-restart.png`
- `regular-wide-split-pdf.png`
- three driver logs, runner stdout/stderr, and `result.json`

## Harness corrections during the run

Two failures were observation-harness defects, not product failures:

1. A standalone run collided with an already active shared WebKit session and
   then timed out on the shared display. The authoritative rerun used the
   registered release runner, which supplies isolated ports, Xvfb, and D-Bus.
2. WebKitGTK once hung on a second full-window WebDriver screenshot. Retained
   drawer screenshots now capture the actual isolated X11 surface with ImageMagick
   `import`, matching the established native E2E practice and avoiding transport
   flakiness. The 1600 px wide-neighbor receipt uses WebDriver capture because
   the isolated Xvfb root is only 1280 px wide. A later run exposed the impossible-width PDF assertion described
   above; it was corrected without weakening the 960 px simultaneous-sidebar
   assertion or changing product code.

Cleanup self-check found no process from the final isolated runner. A separate
pre-existing shared `tauri-driver`/Tine pair on ports 4582/4583 (the source of
the first standalone collision) was deliberately left untouched because it is
owned outside this scenario.
