# GH #161 E1 browser drawer proof

Date: 2026-07-16  
Worktree: `/aux/koutecky/logseq/tine-agent-worktrees/batch-reference-and-polish-v0510`  
Scope: manager checklist row E1 only  
Status: deterministic Chromium acceptance green; no commit created

## Observation boundary

The prior `scripts/check-mobile-drawers.mjs` was a left-only static HTML fixture:
it fabricated the dialog, scrim, and classifier data attributes itself and then
observed only production CSS. It could not prove production controller or
classifier behavior.

The replacement starts a local Vite server and runtime-imports:

- `src/components/MobileDrawerShell.tsx` (`DrawerBackground`,
  `MobileDrawerPanel`, and `MobileDrawerController`);
- `src/mobileDrawers.ts` and `src/ui.ts` for the real media-query classifier,
  active-drawer derivation, exclusivity, dismissal, and state transitions;
- `src/styles/theme.css` and `src/styles/app.css` as executable production CSS.

It mounts those production primitives in the narrow shell topology they own.
It does not mock or invent a backend. It also does not claim to mount the whole
`App`, real `Sidebar`, or real `RightSidebar` contents: their mounted semantics
remain assigned to `src/mobileDrawers.shell.test.tsx`, the sidebar/right-editor
render matrices, and E2's actual Tauri/WebKit process. The retained proof JSON
records SHA-256 fingerprints for all five runtime-imported production inputs.

## Literal browser matrix

The checker passed six fresh Chromium contexts:

| Width/profile | Classifier | Observed geometry |
|---|---|---|
| 639, desktop/fine/no touch | drawer | left and right fixed overlays, each 595 px; workspace remained x=0, width=639 |
| 639, mobile/coarse/touch | drawer | non-zero safe areas top=7/right=17/bottom=9/left=11; exact cap 567 px; left x=11, right edge=622 |
| 640, desktop/fine/no touch | persistent | both sidebars coexist; left 220 px and right 260 px consume flex width |
| 640, mobile/coarse/touch | persistent | same persistent two-sidebar geometry despite mobile/touch configuration |
| 1024, desktop/fine/no touch | persistent | both sidebars and all resize seams available |
| 900, mobile/coarse/touch tablet | persistent | same persistent behavior despite coarse pointer, touch, and mobile UA |

The 639 fine-pointer run also changes the live viewport to 640 and back to 639
without remounting. At 640 the scrim/modal/inert state is removed and both
persistent panels are present. Returning to 639 re-enters drawer mode through
the production media-query listener, normalizes simultaneous state to the
specified right-wins drawer, restores full workspace width, and leaves one
scrim.

For both compact profiles and both drawer sides, the checker proves:

- fixed edge-overlay geometry leaves the underlying workspace rectangle
  unchanged;
- the production panel supplies `role="dialog"`, `aria-modal="true"`, and its
  accessible label;
- the correct production background regions and ordinary floating region are
  `inert`, while the active panel is not;
- the active sidebar resizer and window resize grip compute to `display:none`;
  across the two exclusive sides this covers left, right, and window resizers;
- exactly one production controller scrim exists;
- a real underlying button at the tested coordinates is clicked once before
  opening to prove its activation spy is live, then the same coordinates hit
  the scrim after opening; production pointerdown/click handlers both report
  `defaultPrevented=true` and `cancelBubble=true`, the drawer closes, and the
  underlying activation count remains zero;
- left-to-right and right-to-left switching leaves only the selected panel,
  one scrim, and the correct inert ownership; final close removes every panel,
  scrim, and drawer-added inert state.

At 640 and the wide neighbors, both panels are relative/flex sidebars, both may
remain open, their resizers and the window grip are visible, no scrim or inert
state exists, no panel has modal dialog attributes, and the workspace rectangle
shrinks by the persisted sidebar widths.

## Retained artifacts

Bounded artifacts are retained under:

`test-results/issue-161-browser-drawers/`

- `proof.json` — full six-profile measurements, hit-test/prevention results,
  transition cleanup, and production-input fingerprints (about 80 KiB);
- `639-fine-left.png`;
- `639-coarse-touch-right-safe-area.png`;
- `640-coarse-touch-persistent.png`;
- `1024-fine-persistent.png`.

All four screenshots were visually inspected after the green run. They show the
left/right capped overlays and dimming at 639, including the inset right drawer,
and the two simultaneous width-consuming sidebars without dimming at 640/1024.

## Files owned by E1

- `scripts/check-mobile-drawers.mjs` — production-module/CSS browser harness,
  assertions, proof JSON, and bounded screenshots.
- `package.json` — only `ui:check-mobile-drawers` was added.
- `evidence/2026-07-16-issue-161-browser-drawer-proof.md` — this receipt.

No production source, regression catalog, changelog, E2/native script, or other
manager row was edited.

## Verification

```text
rtk proxy node --check scripts/check-mobile-drawers.mjs
=> passed

rtk npm run ui:check-mobile-drawers
=> PASS: GH #161 browser drawer matrix (6 profiles)
=> artifacts: test-results/issue-161-browser-drawers
```

## Residual risk

Chromium proves CSS layout, real pointer hit-testing, safe-area env handling,
and the imported Solid controller/classifier behavior, but it is not Linux
WebKit and does not drive the complete production `App` or native window. Real
Tauri DOM, WebKit focus behavior, route/editor persistence, and native process
geometry remain E2 obligations. This E1 result must not be presented as native
WebKit or Android hardware evidence.
