// Canonical projection comparator — the SINGLE source of truth for "are two
// projections (lsdoc vs the mldoc oracle) equal?". Every gate / fuzz / tripwire
// script imports `canonJSON` from here so the ignore-set can no longer drift: it
// used to be copy-pasted across 7 scripts and HAD drifted (inlinegate carried no
// ignore filter at all, and `aligns` had to be hand-patched into 4 of them one by
// one in v0.2.3).
//
// Object-key order is irrelevant (serde vs JS emit different orders), so equality
// is decided on a key-sorted canonical JSON string with IGNORE_KEYS dropped.
//
// IGNORE_KEYS — keys excluded from comparison (verified by lsdoc's own tests, not
// the oracle):
//   span — mldoc's block spans are quirky/inconsistent (a Src swallows trailing
//   blank lines, a Property_Drawer doesn't; lone blank lines become paragraphs)
//   and mldoc emits NO inline spans at all. Per SPEC §5 we don't bind to mldoc's
//   internal node identity. See DECISIONS.md ("Spans excluded from comparison").
//   aligns — lsdoc-only table column alignment (`:--`/`--:`/`:-:`), an enrichment
//   for `render_html`'s `data-align`. mldoc 1.5.7 discards alignment, so it has no
//   such field; dropping it (like `span`) keeps the byte-exact gate unaffected.
//   span_map — lsdoc-only exact text-to-source segments for transformed `plain`
//   nodes; validated by harness/spans.mjs alongside `span`.
export const IGNORE_KEYS = new Set(["span", "aligns", "span_map"]);

// Recursively sort object keys so comparison is order-insensitive, dropping
// IGNORE_KEYS. Behaviorally identical to the historical per-script `canon()`.
export function canon(v) {
  if (Array.isArray(v)) return v.map(canon);
  if (v && typeof v === "object") {
    const o = {};
    for (const k of Object.keys(v).sort()) {
      if (IGNORE_KEYS.has(k)) continue;
      o[k] = canon(v[k]);
    }
    return o;
  }
  return v;
}

// The comparison key two projections must share to be considered equal.
export const canonJSON = (v) => JSON.stringify(canon(v));
