# GH #161 P1D1-C lifecycle implementation

## Contract and pins

Test-only preservation packet for real-owner reactivation, listener identity
cleanup, same-id stale-disposer safety, false-top consumption, and Lightbox
Escape/Back peeling.  Production and catalog files were not edited; the catalog
remains `fixing`.

```text
src/transientLayers.ts                                    cb08140530814899891d0d333ee8a252a20e60a3
src/components/Settings.tsx                               d3cc8d1fabce7174361b5f190fcd910bb6d63aca
src/components/transientRegistry.p1d1.hierarchy.test.tsx ddfabea1d0291358d70487c1fc5611f69e62f9c0
src/components/transientRegistry.p1d1.focus.test.tsx     aee51e3870df072c0bf47afce974f4232fc21157
tests/ui-regressions/catalog.json                         ae6251c8c190f5608e4fd1161e5b7655e6922396
```

Owned files:

- `src/components/transientRegistry.p1d1.lifecycle.test.tsx`
- this receipt

## Guard inventory

1. Real Find focus reactivation and real Help inside-pointer reactivation: two
   live owners immediately before a `window` Escape through the installed global
   dispatcher; consumed/default-prevented Escape closes only the reactivated
   owner.
2. Ordinary real Find close and real Help component disposal: exact captured
   registry `focusin`/`pointerdown` document handler identities, capture `true`,
   and matching removals; reconnected old roots cannot displace/dismiss a fresh
   lower owner. Spies are explicitly restored.
3. Same-id old disposer/root cannot affect a replacement under a newer
   sentinel; replacement focus and pointer overtake it, then dismiss exactly
   once and restore only the replacement trigger.
4. A false/stale mounted top consumes and prunes the first global Escape; the
   second reaches the lower owner.
5. Real Lightbox menu, Lightbox, then lower owner peel under global Escape;
   a fresh Lightbox repeats menu, Lightbox, lower states under `back`.

Every test explicitly unregisters sentinels/owners or disposes mounted
components before the final `clearTransientLayersForTest()` assertion reset.

## Commands and observations

- Normal exact render list: **PASS**, 9 files / 52 tests.
- No-isolation verbose seed `161`: **FAIL**, 46 passed / 6 failed. Actual file
  order: QuickSwitcher, drawer-focus, transientDispatch, hierarchy, Settings,
  globalCapture, focus, Settings-recording, lifecycle. Failures: the three
  Settings search/advanced assertions, B's real-Advanced focus marker,
  Settings-recording keycap lookup, and lifecycle's Help-pointer top assertion
  (`in-page-find`, not `help`).
- No-isolation verbose seed `42`: **FAIL**, 46 passed / 6 failed. Actual file
  order: Settings-recording, transientDispatch, globalCapture, lifecycle,
  Settings, focus, drawer-focus, QuickSwitcher, hierarchy. All six lifecycle
  guards passed. Failures: the three Settings assertions, B's real-Advanced
  focus marker, and two A hierarchy assertions.
- Exact node neighbors: **PASS**, 2 files / 26 tests.
- Scoped lifecycle `git diff --check`: **PASS**.
- `npm run check:regressions`: **PASS**, 131 entries / index valid.

## Verdict: BLOCKED

The approved production/test pins still match. The normal render list and all
six lifecycle guards pass; seed 42 also proves the new file cleans up under
no-isolation. Both required full no-isolation lists fail in pre-existing
Settings/P1D1-A/B neighbors, and seed 161 additionally makes the new Help
pointer guard fail after that contaminated sequence. This packet may not alter
those production or neighbor tests. No production defect was repaired or
claimed; fresh scoped ownership is required to make the exact shuffled gates
green before P1D1-C can be marked complete.
