// Line presentation of the shared Rust block-region door (I-12). No recognizers.
import { blockRegions, parserReady } from "../render/parse";
import { utf8ByteToUtf16Offset } from "../render/spans";
import type { Format } from "../render/ast";

export function literalBlockOfLine(raw: string, format: Format = "md"): number[] {
  const lines = raw.split("\n");
  // Before init, edits must refuse. This mask conservatively treats all bytes as
  // literal; editBlock supplies the visible refusal for structural mutations.
  if (!parserReady()) return lines.map(() => 1);
  const literal = new Array<boolean>(lines.length).fill(false);
  const starts = [0];
  for (let i = 0; i < raw.length; i++) if (raw[i] === "\n") starts.push(i + 1);
  for (const [start,end] of blockRegions(raw,format).literals) {
    const a = utf8ByteToUtf16Offset(raw,start);
    const b = utf8ByteToUtf16Offset(raw,Math.max(start,end-1));
    for (let i = 0; i < starts.length; i++) {
      if (starts[i] <= b && (i+1 === starts.length || starts[i+1] > a)) literal[i] = true;
    }
  }
  let run = 0;
  return literal.map((on,i) => on ? (i > 0 && literal[i-1] ? run : ++run) : -1);
}
