# GH #161 responsive shell workstream receipt (W2 / S1 / S2)

Date: 2026-07-16  
Worktree: `/aux/koutecky/logseq/tine-agent-worktrees/batch-reference-and-polish-v0510`  
Status: implementation complete for the responsive-shell workstream; final
browser/native geometry evidence remains owned by E1/E2.

## Contract implemented

- Geometry is selected only by the shared `<640px` classifier. In compact mode
  the existing left/right signals normalize deterministically (right wins a
  restored dual-open state); at `>=640px` both persistent sidebars may remain
  open.
- Both production panels share one dialog/Tab contract. The App mounts one
  controller for the width listener, state normalization, focus entry/
  containment, and exactly one consuming scrim.
- Ordinary shell regions use native `inert`: the whole main shell behind a left
  drawer, and top chrome plus pane/PDF workspace behind a right drawer. Mobile
  editor-toolbar buttons, toast actions, focus-mode exit and native resize grips
  are isolated as ordinary floating chrome too. The active drawer and global
  transient siblings remain interactive. A registered
  local transient whose root is inside an inert background does not incorrectly
  suspend drawer focus; a real higher transient does.
- Explicit/Escape closure restores a connected opener; scrim/navigation/Back
  target the active main pane. A disconnected opener falls back safely. Compact
  opener state is cleared when the viewport returns to persistent mode.
- Compact CSS makes either sidebar fixed overlay geometry, caps the persisted
  width so 44px remains exposed (including safe-area insets), hides sidebar and
  frameless-window resizers, and leaves persistent-width styles untouched.

## Owned implementation surfaces

- `src/components/MobileDrawerShell.tsx` (new mounted production primitives)
- `src/mobileDrawers.ts` (focusability, higher-transient and fallback helpers)
- `src/mobileDrawers.shell.test.tsx` (replaced source-string proxy with mounted
  behavior)
- `src/App.tsx` (narrow shell wiring; preserves concurrent N1/App work)
- `src/components/RightSidebar.tsx` (shared production panel/explicit close)
- `src/styles/app.css` (compact overlay/cap/resizer/floating-background rules)
- `src/ui.ts` (only W2 opener cleanup plus review of the existing exclusivity
  normalization; concurrent R1 changes in this file are not claimed here)

## Verification on the recombined moving worktree

Green after the concurrent R1 seam became stable:

```text
rtk proxy npx vitest run --config vitest.render.config.ts \
  src/mobileDrawers.shell.test.tsx \
  src/keybindings.p1a1.drawerFocus.test.tsx \
  src/components/QuickSwitcher.p1c-q.test.tsx \
  src/components/RightSidebar.test.tsx \
  src/components/Sidebar.test.tsx
=> 5 files, 14 tests passed

rtk proxy npx vitest run \
  src/mobileDrawers.test.ts \
  src/components/Sidebar.graphSwitcher.test.ts
=> 2 files, 8 tests passed

rtk proxy npx tsc --noEmit --pretty false
=> passed, no diagnostics

rtk npm run build
=> production Vite build passed (268 modules)

rtk git diff --check
=> passed
```

Bounded manager-review follow-up after isolating `MobileKeyboardToolbar` and
`Toasts`, and adding dialog-root Tab coverage:

```text
rtk proxy npx vitest run --config vitest.render.config.ts \
  src/mobileDrawers.shell.test.tsx \
  src/keybindings.p1a1.drawerFocus.test.tsx \
  src/components/RightSidebar.test.tsx
=> 3 files, 8 tests passed

rtk proxy npx tsc --noEmit --pretty false
=> passed, no diagnostics

rtk git diff --check
=> passed
```

The mounted shell matrix covers compact left/right switching, persistent dual
open, restored dual-open right-wins, reactive compact/persistent transitions,
dialog semantics, inert ownership and cleanup (including the mobile-toolbar and
toast-action regions), one scrim, focus entry, Tab and Shift-Tab wrap from both
the first/last controls and the focused `tabindex=-1` dialog root,
higher-transient suspension, inert-local-transient rejection,
connected/disconnected opener outcomes, and scrim consumption without invoking
the background target.

## Honest remaining proof boundary

jsdom does not calculate flex/fixed layout or pointer hit-testing. Therefore the
literal unchanged-workspace rectangle, edge anchoring/cap, resizer visibility,
and coordinate-level click shield still require the separately assigned E1
Chromium proof and E2 Linux WebKit run. This workstream does not claim R1/N1,
Android Back, transient migration, or those E2E rows.
