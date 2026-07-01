// Typographic replacements (a Tine opinion, NOT in lsdoc / not OG-parity).
//
// Turns ASCII arrow/dash sequences into their proper glyphs at RENDER time:
// the Markdown source keeps `->` / `--` / `---`, and only the rendered (non-
// editing) view shows `→` / `–` / `—`. This mirrors how `\Delta`→Δ already works
// (source-preserving, applied on render) and replaces the old Inter `calt` font
// ligature — which only worked in-app and also caused the contextual-asterisk
// height glitch, so `calt` is now off.
//
// Kept as a pure function (no Solid deps) so the anti-drift gate can apply the
// same transform to lsdoc's reference HTML and stay green: this is a *sanctioned*
// divergence from the lsdoc render contract, applied equally on both sides.
//
// Longest-match-first ordering matters (`<-->` before `<->`, `-->` before `--`,
// `---` before `--`) so combined runs map to one glyph instead of being eaten
// piecemeal (e.g. `-->` → `⟶`, not `–>`).

// NOTE: avoid the emoji-presentation arrows (U+2194 ↔, U+2195 ↕, …) — Tine renders
// those as Twemoji SVGs (a blue box), not text. The long math arrows (U+27F5–27FA)
// are text-only, and read nicely: single-hyphen ⇒ short (→ ←), multi ⇒ long.
const TYPO_MAP: Record<string, string> = {
  "<-->": "⟺", // U+27FA long left-right double
  "<->": "⟷", // U+27F7 long left-right (NOT U+2194 ↔ — that's an emoji)
  "<--": "⟵", // U+27F5
  "-->": "⟶", // U+27F6
  "<-": "←", // U+2190
  "->": "→", // U+2192
  "=>": "⇒", // U+21D2
  "---": "—", // U+2014 em dash
  "--": "–", // U+2013 en dash
};
const TYPO_RE = /<-->|<->|<--|-->|<-|->|=>|---|--/g;

/** Replace ASCII arrow/dash sequences with their glyphs. Pure and idempotent-ish
 *  (the outputs contain none of the input triggers, so a second pass is a no-op). */
export function typographic(text: string): string {
  if (!text) return text;
  return text.replace(TYPO_RE, (m) => TYPO_MAP[m] ?? m);
}
