// Emoji as bundled Twemoji SVG *images*, not a color-emoji font. WebKitGTK paints
// a color-emoji webfont as a blank glyph, so page icons / emoji showed as empty
// gaps; an <img> renders identically in every engine. The SVGs are copied to
// `dist/twemoji/<codepoint>.svg` at build (see vite.config.ts).

import { For, type JSX } from "solid-js";
import emojiRegex from "emoji-regex";

// Twemoji filename rule (its `grabTheRightIcon`): codepoints joined by '-', with
// the U+FE0F variation selector dropped UNLESS the sequence is a ZWJ (U+200D)
// sequence.
export function twemojiName(emoji: string): string {
  const hasZwj = emoji.indexOf("‍") >= 0;
  const cps: string[] = [];
  for (const ch of emoji) {
    const cp = ch.codePointAt(0)!;
    if (!hasZwj && cp === 0xfe0f) continue;
    cps.push(cp.toString(16));
  }
  return cps.join("-");
}

const RE = emojiRegex();
// Coarse "might contain an emoji" guard so plain Latin/Han prose skips the precise
// (slower) scan. No false negatives over the emoji ranges (a few CJK-punctuation
// false positives just cost an extra scan).
const HINT = /[‼-㊙️⃣]|[\u{1f000}-\u{1faff}]/u;

export type EmojiPart = { t: "text"; v: string } | { t: "emoji"; v: string };

/** Split a string into plain-text and emoji runs (pure; unit-testable). */
export function emojiSplit(text: string): EmojiPart[] {
  if (!text || !HINT.test(text)) return [{ t: "text", v: text }];
  const out: EmojiPart[] = [];
  RE.lastIndex = 0;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = RE.exec(text))) {
    if (m.index > last) out.push({ t: "text", v: text.slice(last, m.index) });
    out.push({ t: "emoji", v: m[0] });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ t: "text", v: text.slice(last) });
  return out.length ? out : [{ t: "text", v: text }];
}

/** Render text with any emoji turned into Twemoji <img> SVGs. */
export function EmojiText(props: { text: string }): JSX.Element {
  return (
    <For each={emojiSplit(props.text)}>
      {(p) =>
        p.t === "text" ? (
          <>{p.v}</>
        ) : (
          <img
            class="emoji"
            draggable={false}
            alt={p.v}
            src={`/twemoji/${twemojiName(p.v)}.svg`}
            onError={(e) => {
              // No bundled SVG (a very new emoji) → fall back to the raw glyph.
              const span = document.createElement("span");
              span.textContent = p.v;
              e.currentTarget.replaceWith(span);
            }}
          />
        )
      }
    </For>
  );
}
