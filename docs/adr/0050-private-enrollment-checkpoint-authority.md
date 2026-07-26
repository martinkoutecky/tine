# 0050. Private enrollment checkpoint authority

- **Status:** Accepted
- **Date:** 2026-07-26

## Context

Enrollment history is immutable and unbounded, while opening an enrollment must
perform fixed-bounded work. A recursive public digest is not a proof of the
unread prefix: arbitrary record bytes can recompute it after inserting an illegal
transition. Stopping after a fixed number of links therefore cannot establish
lifecycle lineage.

## Decision

Each enrollment provisions one versioned private authority claim in the trusted
application-data namespace. The claim contains a random 256-bit HMAC-SHA256 key
and is bound to the exact enrollment binding and retained lease resource. Its OS
file identity is also bound into authenticated records.

Generation 1 and every 64th-generation boundary thereafter carry an HMAC
checkpoint over the exact record state. A writer may mint the next checkpoint
only after opening or constructing the complete legal suffix from a previously
authenticated checkpoint. Open validates at most 64 records and must terminate
at a scheduled checkpoint accepted by the local authority. Audit remains paged
and validates all links, including links spanning opaque cursor boundaries.

Authority provisioning uses create-new publication plus directory durability.
Only one canonical, exactly bound authority temporary file is resumable; missing,
foreign, substituted, multiple, or incompatible material fails closed and is
preserved.

## Threat model and consequences

This prevents arbitrary enrollment record bytes from minting a trusted history
summary. It does not protect against a process running with the same user
authority that can read the private key and rewrite the entire private store.
Filesystem atomicity and directory-sync guarantees remain platform dependent.

The record format advances to schema version 3. History remains immutable, open
work has a fixed 64-record bound with no finite lifetime, and the new authority
claim becomes required after enrollment publication.
