# Contributing to Tine

Thanks for your interest. Tine is a **single-maintainer project**, and the way it
takes contributions is deliberately a little unusual — please read this before
opening a pull request.

## The most valuable contributions

1. **Testing on your setup and reporting back.** Tine targets Linux (primary),
   macOS, and Windows, across many distros, GPUs, and Logseq graphs. Real-world
   reports — "it won't start on Wayland + NVIDIA", "this `.org` page loads
   read-only", "rename mangled a `#tag` inside a URL" — are genuinely the highest-
   value thing you can send. Use the **bug report** form; for a startup/crash
   problem, a `TINE_DEBUG=1` log (see the README) is usually enough to diagnose it.
2. **Feature requests and design proposals.** See below — for Tine, a well-described
   proposal *is* the contribution.

## How code changes work here: propose, don't patch

**Tine does not merge externally-written code into the app.** Instead of a code PR,
open an issue that describes the change as a **specification**:

- the problem / desired behavior,
- the key **design decisions** (where data is stored, what the format is, how it
  behaves at the edges, how it interacts with Logseq round-tripping),
- and how Logseq does it, if it mirrors a Logseq behavior.

The maintainer then implements it (often AI-assisted) under direct review, and
credits you as the proposal's author.

**Why this way?** Tine reads and writes your real notes with your filesystem
privileges, and a single maintainer cannot honestly security-review arbitrary
incoming code. Reviewing a *human-readable design* is tractable in a way that
reviewing an obfuscated diff is not — a risky decision ("cache rendered notes to a
world-readable file") is visible in a spec. It also keeps provenance clean and
leaves an auditable trail of *why* (see [`docs/adr/`](docs/adr/)). The honest limit:
this shrinks the attack surface to design decisions a maintainer can read and
reason about — it does **not** make every security-relevant choice obvious, so
proposals touching the **filesystem, network, credentials, or config** get extra
scrutiny. Flag those explicitly.

The trade-off is real: a perfectly good patch you wrote still has to be
re-described and re-implemented, which is slower and gives the contributor's code
itself no direct path in. For a project whose bottleneck is maintainer review time,
that's the honest order of things.

### The exception: docs and trivial fixes

Documentation fixes, typos, and other **non-code** changes can come straight in as
an ordinary pull request — use the PR template.

## Licensing

Tine is licensed **[GNU AGPL-3.0-only](LICENSE)**. By contributing — whether a
proposal, a docs PR, or a rare accepted patch — you agree your contribution is
licensed under AGPL-3.0 (inbound = outbound). There is **no CLA**. If you like, sign
off your commits (`git commit -s`, a [DCO](https://developercertificate.org/)
`Signed-off-by` line); it's welcome but not required.

## Why things are the way they are

The architecture (Tauri/WebKitGTK over Electron, a pure-Rust core, in-browser WASM
parsing, the data-safety invariants, operating directly on the Logseq format) is
documented as short decision records in **[`docs/adr/`](docs/adr/)**. If a proposal
runs against one of those, that's worth saying out loud in the issue.
