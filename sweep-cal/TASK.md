# Sweep reader task — calibration run, bundle B012

You are one reader in an exhaustive sweep of Tine's production code. Your
assigned bundle is exactly ONE file of this repository (your working directory
is the repo root):

- crates/tine-core/src/oplog/operational_coordinator.rs  (3182 lines)

## Setup

Read `sweep-cal/INVARIANTS.md` IN FULL first — Parts 1 and 2 tell you what Tine
is and what matters most; Part 3 holds the 22 invariants (each with a Rule, a
Specimen, and the Ask you must answer); Part 4 defines the W and P channels;
Part 5 the verdict schema.

## How to read

- **Read the assigned file COMPLETELY, top to bottom.** Do not skim, do not
  sample, do not grep-and-declare. A sampled verdict is worthless.
- You may grep/read OTHER files in this repository to resolve a specific
  question (who calls this? does that type exist?), but your verdicts cover
  ONLY the assigned file.
- **Report facts, not bug judgments.** A VIOLATION verdict means "this file
  does the thing the Rule forbids, here" — whether it is acceptable is the
  manager's triage, not yours.
- `CANNOT-DETERMINE` is a first-class honest answer. Never guess.
- Do not trust comments or docs — that is invariant I-11. A comment's claim is
  a thing to check, not a fact.
- Inline `#[cfg(test)]` modules inside the file are out of scope for verdicts
  (note their line range on the `read:` line) — except under I-11, where a test
  enforcing a claim is exactly what you cite.

## Output

Write your verdicts to `sweep-cal/VERDICT-B012-kimi.md` in the repo, following
the exact Part 5 schema from INVARIANTS.md: the four-line header (bundle:
B012-kimi / commit: the output of `git rev-parse HEAD` / invariants: v1.2 /
date: today), then one `=== FILE:` block for the assigned file with `purpose:`,
`read:`, all 22 verdicts I-1..I-22 in order, then `W:` and `P:` entries.
Verdict vocabulary: N/A | PASS — <what you checked> | VIOLATION — file:line —
<sentence> | CANNOT-DETERMINE — <what you would need to know>. A bare PASS with
no sentence is invalid. Multiple sites for one invariant: worst site on the
verdict line, the rest as indented `  - file:line — sentence` continuation
lines.

## Hard constraints

- Change NOTHING in the repository except creating `sweep-cal/VERDICT-B012-kimi.md`.
- Do not run builds or tests; this is a reading task.
- When done, print a 5-line receipt: bundle id, file read, count of
  VIOLATION / CANNOT-DETERMINE / W / P entries, and the single most important
  thing you found.
