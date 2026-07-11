# Community plugin registry policy (v0)

The catalogue welcomes human-written, AI-assisted, and AI-primary (“vibecoded”)
plugins. AI provenance is disclosed for transparency but is neither approval nor a
risk score. Safety is based on authority, containment, reproducible evidence, and
revocation.

## Submission requirements

- public source repository and a recognized open-source SPDX license;
- immutable release commit/tag, `manifest.json`, WebAssembly entry, and SHA-256;
- explicit capabilities and platforms (omission means desktop only);
- deterministic `tine-plugin-check/v1` report;
- successful clean build in the registry's hostile-build container;
- automated source review report with evidence and an uncertainty disposition;
- no telemetry or analytics sent to Tine or Martin; API 0.1 has no network anyway.

The category “community” is about distribution and core commitment, not who typed the
code. Martin may publish a community plugin to answer a feature request quickly. That
does not promise that the behavior will enter core or that the plugin will be kept
forever. Tine Labs is separate and reserved for experiments plausibly headed to core.

## Automated dispositions

- **Published:** deterministic checks pass, build is reproducible, only low-risk
  ordinary capabilities are used, and automated review reports no uncertainty.
- **Quarantined:** any check fails, graph-write authority needs unresolved review,
  source/binary mismatch exists, or the reviewer is uncertain. Quarantine is the
  safe default and requires no maintainer attention.
- **Rejected:** policy/license/source requirements are absent or malicious intent is
  supported by evidence.
- **Revoked:** a previously published immutable version is unsafe. The signed index
  records reason, time, replacement (if any), and severity; Tine disables it.

Reports are public beside the version and distinguish deterministic facts from AI
judgment. Passing review is not a guarantee. The app always shows identity,
capabilities, platforms, digest, audit age/status, and source before enablement.
