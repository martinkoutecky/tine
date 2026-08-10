# 0053. Enrollment checkpoint integrity and legacy verification

- **Status:** Accepted
- **Date:** 2026-08-10
- **Supersedes:** [0050](0050-private-enrollment-checkpoint-authority.md)

## Context

Enrollment needs a bounded, canonical way to detect corrupted, substituted, or
structurally illegal history before it admits a writer. The original format
used a same-user private HMAC key for checkpoints. That is useful for opening
old stores, but it does not establish a security boundary against a process
that can replace the entire local private store. Calling it a private trust
authority therefore overstates the guarantee.

The historical bytes must nevertheless remain readable. A migration must not
rewrite an existing `authority-v1.claim`, alter its file identity, or invalidate
its v5 record suffix.

## Decision

New enrollment authority claims use schema v2 and contain only the canonical
authority identity, lease resource, exact binding, and initial preparation
intent. They retain the `authority-v1.claim` path because that path identity is
part of the lease protocol.

New enrollment records use schema v6 and their scheduled checkpoints use
schema v3. The checkpoint message is canonical JSON over the authority and
resource identities, generation, predecessor and history accumulator, lease,
binding, and lifecycle. Its CRC-32/ISO-HDLC tag detects accidental corruption
and mismatched structural identity. The same style of keyless CRC integrity is
used for process-local audit cursors.

The legacy v1 authority claim, v5 record, and v2 HMAC checkpoint are frozen
verification-only codecs. A v1 authority can therefore reopen an existing v5
history and lazily append v6 successors; it never mints another HMAC
checkpoint. If an old binary crashed before publishing the first HEAD, a
current writer may finish exactly one canonical, correctly bound and
HMAC-verified v5 generation-one candidate under the existing capability and
link-count rules, without rewriting its authority or record bytes. Readers
accept a mixed chain only when each record/checkpoint pair uses its exact
matching codec. Generation 1 and each 64-record boundary remain the required
checkpoints, so bounded open and paged complete audit keep their existing
limits.

## Consequences

Enrollment continues to reject malformed, noncanonical, mismatched, or
structurally illegal data before lifecycle admission. The new format makes a
corruption-detection and canonical-identity/lease/lifecycle proof claim, not a
claim of cryptographic protection from a same-user complete-store rewrite.
Existing v1/v5 stores remain compatible without a migration rewrite, while all
fresh state and all new suffixes converge on the current keyless format.
