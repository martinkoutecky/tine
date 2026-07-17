# GH #161 P1D2-F — implementation receipt

## Contract and boundary

- Tier: medium. The approved P1D1 registry and capture dispatcher remain the
  authority; this packet changes only the target-local Escape fallbacks at the
  mounted InPageFind and PageProps text inputs.
- Contract: composing/keyCode-229 Escape remains unprevented and makes no
  transient dismissal. Find keeps its ordinary bubbling behavior; PageProps
  preserves its established target stop-propagation boundary before the IME
  return. With global capture installed, ordinary Escape consumes one top
  registered owner before the target handler.
  Without it, the target handler calls `dismissTopTransient("escape")` once and
  prevents default only when that operation handled an owner. It performs no
  independent target close.
- Excluded: registry/keybindings mechanics, P1D1 tests, other fallback owners,
  drawers/Android, P2-P5, commit, GitHub, deploy, and release.

## Pinned inputs and candidate hashes

Approved input blobs:

- `src/components/InPageFind.tsx` `b5ef6e2cad2760a790e5b05f5165b2dcf9437cd1`
- `src/components/PageProps.tsx` `b888fc8ad39f34d52a26abd49a987dac3aa88831`
- `src/components/HelpShortcuts.tsx` `a1125baacd68c7800ce1a472800f34861d30791d`
- `src/transientLayers.ts` `4bac1dda8a01329c54e17f3e2f145bc50e84156b`
- `src/keybindings.ts` `2d1b04d05a39012f00da47cee98f0b2b7692612f`
- `vitest.render.config.ts` `0244dd891977eba60778ea77f505127d48d911e0`
- `src/testSetup.render.ts` `eb2502cb629b6b4504b95d754fb89d19a1518c5c`

Candidate output blobs:

- `src/components/InPageFind.tsx` `9beeba6c1b6f85fac7cc1f38ad4c1c3049789433`
- `src/components/PageProps.tsx` `8691ab78ea7c87c2a79aac176edb558c7434dc30`
- `src/components/transientFallbacks.p1d2.test.tsx` `87a600581ff7bc7a17efb4b99c7c9786da6b0a55`
- `tests/ui-regressions/catalog.json` `ab4fc67b872d5d5720cec05e3c1265ff126c8dd6`

Owned files are the two target components, the new mounted test, the single
catalog entry, this receipt, and the external raw fail-before receipt. The
catalog entry `UI-TRANSIENT-INPUT-FALLBACK-ORDER-001` (GH #161) was added as
`reproduced` after fail-before and is now `fixing`; the broader GH #161 entry
was not changed.

## Necessity evidence

Before production edits:

```text
rtk proxy npx vitest run --config vitest.render.config.ts src/components/transientFallbacks.p1d2.test.tsx
```

Exited 1: 2/7 tests failed. Both literal mounted no-global paths left Help open
after Escape at the Find/PageProps input, proving the current direct target
close bypassed the newer registered owner. Raw output:
`/aux/koutecky/logseq/tine-agents/specs/implementation/evidence/2026-07-16-issue-161-p1d2-fallbacks-fail-before.txt`.

## Candidate verification

- `rtk proxy npx vitest run --config vitest.render.config.ts src/components/transientFallbacks.p1d2.test.tsx src/components/transientRegistry.p1d1.hierarchy.test.tsx src/components/transientRegistry.p1d1.focus.test.tsx src/components/transientRegistry.p1d1.lifecycle.test.tsx src/components/transientDispatch.p1.test.tsx src/keybindings.p1a1.globalCapture.test.tsx` — 6 files, 49 tests passed.
- Seed `1612`, exact no-isolation command from the packet — 6 files, 49 tests passed. Actual file order: `transientRegistry.p1d1.hierarchy`, `transientDispatch.p1`, `transientRegistry.p1d1.lifecycle`, `transientRegistry.p1d1.focus`, `keybindings.p1a1.globalCapture`, `transientFallbacks.p1d2`.
- Seed `2161`, exact no-isolation command from the packet — 6 files, 49 tests passed. Actual file order: `transientFallbacks.p1d2`, `transientRegistry.p1d1.lifecycle`, `transientRegistry.p1d1.hierarchy`, `keybindings.p1a1.globalCapture`, `transientRegistry.p1d1.focus`, `transientDispatch.p1`.
- `rtk proxy npx vitest run src/keybindings.test.ts` — 1 file, 22 tests passed.
- `rtk git diff --check -- src/components/InPageFind.tsx src/components/PageProps.tsx src/components/transientFallbacks.p1d2.test.tsx tests/ui-regressions/catalog.json` — passed.
- `rtk npm run check:regressions` — passed: UI catalog 132 entries / 108 GitHub issues; regression indexes valid.

## Rejected implementation repair round

The prior candidate was rejected for a PageProps IME propagation regression and
proxy-level adjacent proofs. This bounded repair restores `stopPropagation()`
before PageProps' composing/keyCode-229 return; it leaves both event kinds
unprevented and undismissed. At both real inputs, separately dispatched
composing and 229 Escape now preserve owners, value, caret, and focus with and
without `installKeybindings()`. A real window bubble sentinel observes Find's
existing bubbling and no PageProps bubble, including with the capture listener.

The strengthened observations create a routed named page with two `needle`
matches and drive the real Find input: Enter commits `inPageFindQuery` and
selects index 1, Previous selects 0, and Shift+Enter returns to 1. They create a
writable named page and locate the `alias` field by its explicit
`PAGE_PROP_SPECS` key/label association; actual Enter then actual blur each
produce the corresponding `readPageProperty` value. After reopen, the public
registry reports PageProps as top; after close and after component unmount,
focus/pointer events dispatched at its detached old root cannot replace the live
public-registry sentinel. The catalog remains `fixing`; the original causal
2/7 fail-before receipt is unchanged.

Preserved scope: this repair edits only `src/components/PageProps.tsx`,
`src/components/transientFallbacks.p1d2.test.tsx`, and this receipt. InPageFind
production, transient registry/keybindings, catalog, setup/config, private
specs, GitHub, changelog, and P1D3+ remain untouched. Full
typecheck/repository/E2E gates remain deferred to P1 recombination.
