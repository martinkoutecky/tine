# GH #161 T1/T2 transient completion receipt

## Contract and boundary

- **Tier / authority:** high-risk umbrella implementation workstream under the
  already-approved amended GH #161 specification and persistent-manager
  checklist. No new spec/verifier lifecycle was opened.
- **Pinned base:** `f6f6de13e1180f61ce8d54267d164ffe994eeae7`
  plus the preserved dirty P1A/P1C/P1D1/P1D2/P1E-O candidate described by the
  existing receipts.
- **Outcome:** one non-composing Escape/Back gesture changes at most one live
  transient rung. IME composition and keyCode 229 change none. Registrations
  belong to mounted owners and retire through the same idempotent lifecycle as
  their pointer/session cleanup.
- **Excluded:** responsive classifier/shell geometry, navigation, whole-right
  teardown, Android/native policy, browser/native E2E, GitHub, commits, deploy,
  release, and private process files.

## Implementation

1. Corrected the P1D2 proof seam: the retired PageProps root is reattached to
   `document.body` while a live sentinel remains registered before focus/pointer
   events are dispatched. The proof is no longer vacuous on a detached node.
2. Added the production `WelcomeLayer` truth table. Forced/no-graph Welcome is
   mandatory and unregistered even when the optional signal is true; optional
   Welcome over a graph owns exactly one dismissal. Both transition directions
   update ownership without remount residue.
3. Replaced TabBar's private strip-drag key listener with one armed, global,
   generation-safe transient session. Escape/Back, replacement, pointer cancel,
   lost capture, blur, tab disappearance and component disposal share the same
   idempotent finish seam; late retired-pointer release cannot finish the newer
   session. Overview ownership from P1E-O is retained.
4. Replaced SheetBoard's private drag Escape listener with the board
   coordinator's real mounted transient owner. All existing finish paths dispose
   the same registration before releasing pointer capture and clearing the
   ghost/body/write state.
5. Made each mounted page/block PeekPopup a unique real-root owner and added an
   immediate bridge dismissal which clears both hover timers. Removed its two
   independent keyboard owners while retaining hover grace, scroll/resize and
   nested-peek behavior.
6. Completed already-listed owner neighbors: Settings conflict content/rename
   and merge modal are semantic children of Settings; existing QuickSwitcher,
   ContextMenu, Block completion, right actions, Lightbox and expanded Audio
   owners remain on the shared dispatcher. Added literal Audio Escape/Back
   lifetime proof; the existing lifecycle matrix already proves Lightbox menu →
   Lightbox → lower ordering for Escape and Back.
7. Added/updated six regression-catalog rows for Welcome, strip drag, board drag,
   Peek, Audio and the earlier input fallbacks. They remain `fixing` until the
   persistent manager freezes and verifies the complete umbrella candidate.

## Literal necessity evidence

Before the Tab/Board/Peek production edits, the implementer-authored mounted
tests were run together. The command exited 1 with 14 failures / 50 passes. The
causal failures were:

- strip drag: the lower sentinel dismissed (`1`, expected `0`); composing and
  keyCode-229 Escape were prevented and canceled the live drag;
- board drag: the lower sentinel dismissed; composing/229 canceled the live
  ghost/session;
- page peek: the lower sentinel consumed while the popup stayed visible;
  composing/229 removed the popup; shared `back` returned false.

The additional downstream SheetBoard failures in that raw combined run were
test contamination after those intentional assertions aborted cleanup, not
separate production findings. The pre-existing D2 and Welcome causal evidence
remains in their approved specs/receipts.

## Exact current-tree verification

Passed:

```text
focused mounted pass-after:
  transientFallbacks.p1d2, Welcome.p1d3, TabBar, SheetBoard, pagePeek
  5 files / 77 tests at the first recombination pass

owner neighbors:
  AudioOverlay, transientRegistry lifecycle/hierarchy, Settings,
  Settings recording
  5 files / 18 tests

no-isolation shuffled matrix seed 161:
  13 files / 127 tests

no-isolation shuffled matrix seed 229:
  13 files / 127 tests

final focused corrections:
  Welcome.p1d3 + Welcome + transientDispatch: 3 files / 12 tests
  TabBar: 1 file / 12 tests

node dispatcher neighbor:
  src/keybindings.test.ts: 22 tests

npx tsc --noEmit
git diff --check (owned files)
npm run check:regressions
  UI catalog OK: 137 entries / 108 GitHub issues; both indexes valid
```

Expected jsdom-only diagnostics: Audio tests report the existing unimplemented
Canvas `getContext`; all assertions pass.

Relevant final SHA-256 fingerprints:

```text
1e64c072d8351215fa4f11bd0884a73700b42179e3031fea9c76a9df4771f31f  src/components/TabBar.tsx
e94212049780c7e2800d966827d20cbbf764e9fd2e6f0c9f253c24fbb471fffe  src/components/TabBar.test.tsx
4da20891c4581ce36a2dc52e2ed4f42b8a8ea2ae4fa5eafb4cf94682bb09d7bf  src/components/SheetBoard.tsx
4bfb73d484bc383dddccd0fca684bde0934b2698d67d9a765a8639a130c534b0  src/components/SheetBoard.test.tsx
70100b3b302e11d59830889ecc87c67294c1ea4b5e950e71123b00554acf1482  src/render/PeekPopup.tsx
a49ed6eee3d3dcbd09772d21a3eb7396d6cf019aff77bcebc10b67f1b0e7438f  src/render/inline.tsx
42f7e041b7b3c00309d2da2dbe976d31d87167a26ff5039cb2a836fc6d2be1df  src/render/pagePeek.test.tsx
03ff99ebe2afa57738c555cd722889e66cf597b082a14674d3830b2c4e9e8735  src/components/Welcome.tsx
12e7a3d232078db85c963016f78bb4e871e0288aa087d673f32b8f87c28cdfd0  src/components/Welcome.p1d3.test.tsx
e9dbed8c9b8fda768fce804574ae27b84ef477eada5e359b0f33e43ef9c4a973  src/components/transientFallbacks.p1d2.test.tsx
0f1eb3e084367e4097ace104dd4ed3fb915465abc64f6bff900b915caf1def6c  src/components/Settings.tsx
a766f881a9d5c6165029718413d38bae69b6bb0c0aa0fec8a519f7965aac6ba4  src/components/AudioOverlay.test.tsx
eada63fe94f5778f877513f363acee68e4b0b12ec19d0b3ffbda9d4e2235287f  tests/ui-regressions/catalog.json
```

## Frozen census dispositions

The earlier closure census also named CalendarJump, PDF-local Find/highlight
color, QueryWorkspace Advanced, Formula value pickers, Block selection overflow
and QueryBuilder popovers. None had a literal mounted one-Escape lower-layer
failure in this bounded workstream, and migrating them now would turn T1/T2 into
a generic overlay project. They are classified as **follow-up**, not release
blockers, pending a separate causal repro and proportionality decision.

## Verdict

The explicit T1/T2 completion rows owned by this workstream are implemented and
focused-green. The umbrella remains `implementation incomplete — continue work`
until W/S/R/N/A/E/G workstreams finish and the manager certifies one immutable
candidate. No T1/T2 code row is knowingly incomplete inside this frozen scope;
the census follow-ups above are deliberately excluded rather than silently
claimed fixed.
