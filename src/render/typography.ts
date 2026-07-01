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

// --- "On type" mode: rewrite the SOURCE as you type (typographyMode === "type").
// Called from Block.tsx's onInput after a single char is inserted. The hard part
// is knowing when a trigger is COMPLETE and can't be extended by the next char:
//   • `>`-terminated triggers (`->`, `=>`, `-->`, `<->`, `<-->`) are unambiguous —
//     nothing extends past `>`, so we replace the moment `>` is typed (longest
//     match wins, so `-->` isn't eaten as `->` then a stray `-`).
//   • dash/left-arrow triggers (`--`, `---`, `<-`, `<--`) end in `-`, which a
//     later `-`/`>` could extend into a longer trigger. So we DEFER them: when the
//     NEXT char typed is a boundary (not `-`/`>`), we resolve the run that ended
//     just before it. (A trailing `--` at the very end of a block therefore stays
//     ASCII until another char follows — an accepted, minor conservatism.)
// Both cases are unit-tested in typography.test.ts.

// `>`-terminated triggers, longest first.
const GT_KEYS = ["<-->", "-->", "<->", "->", "=>"];
// Dash/left-arrow triggers resolved on the FOLLOWING boundary char, longest first.
const DEFERRED_KEYS = ["<--", "---", "<-", "--"];

function splice(value: string, from: number, to: number, glyph: string, caret: number) {
  return { value: value.slice(0, from) + glyph + value.slice(to), caret: caret + (glyph.length - (to - from)) };
}

/** On-type typographic replacement for a single inserted char. `value`/`caret`
 *  are the post-input textarea state; `typed` is the char just inserted (at
 *  `value[caret-1]`). Returns the adjusted `{value, caret}` or null.
 *
 *  Skipped inside code: an odd number of backticks before the caret means we're
 *  inside inline code (or past a `\`\`\`` fence opener — 3 is odd), where the
 *  source is literal and must not be rewritten. */
export function typoTypeReplace(
  value: string,
  caret: number,
  typed: string
): { value: string; caret: number } | null {
  let ticks = 0;
  for (let i = 0; i < caret; i++) if (value[i] === "`") ticks++;
  if (ticks % 2 === 1) return null;
  // `>` terminates an arrow → replace immediately (maximal, longest-match).
  if (typed === ">") {
    for (const k of GT_KEYS) {
      if (caret >= k.length && value.slice(caret - k.length, caret) === k) {
        return splice(value, caret - k.length, caret, TYPO_MAP[k], caret);
      }
    }
    return null;
  }
  // A boundary char (not `-`/`>`) confirms a preceding dash/left-arrow run can't
  // extend → resolve the run that ended one char back (just before `typed`).
  if (typed !== "-") {
    const end = caret - 1; // position of `typed`; the run sits in [.., end)
    for (const k of DEFERRED_KEYS) {
      if (end >= k.length && value.slice(end - k.length, end) === k) {
        return splice(value, end - k.length, end, TYPO_MAP[k], caret);
      }
    }
  }
  return null;
}
