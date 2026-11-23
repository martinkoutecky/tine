# 0016. The mock backend is an intentionally lossy dev double

Date: 2026-07-02

## Status

Superseded by [0081](0081-browser-mock-on-the-core.md): the mock now answers no graph question; the browser core does.

## Context

`src/mock.ts` reimplements slices of `refs.rs`/`query.rs`/`model.rs` in TS so
the app runs in a plain browser for dev, screenshots, and vitest (ADR 0002's
"frontend runs headless" consequence). It is the #5 churn file — every backend
behavior change tempts a matching mock change, and the "Mirrors …" comments make
it look like a parallel implementation that must be kept exact.

## Decision

- The mock is a **dev/test double, not a second backend**. It mirrors only what
  the dev/screenshot/test harnesses actually exercise; fidelity beyond that is
  explicitly a non-goal.
- Mock drift can corrupt *screenshots and dev-harness behavior*, never a user
  graph. Bugs reproducible only against the mock are harness bugs.
- Don't accrete authoritative logic into it. If a harness needs real behavior
  (asset-write IPC, org round-trip, conflict flows), the answer is the real
  backend (e2e harness / smoke test in the running app), not a richer mock.

## Consequences

- Reviewers should push back on mock changes that chase backend parity beyond
  harness needs.
- Anything verified only against the mock still needs a real-app check before
  it counts as verified (see the screenshot/e2e docs).
