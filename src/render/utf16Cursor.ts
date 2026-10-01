/** Converts UTF-8 byte offsets of `text` (parser coordinates) to UTF-16 offsets by walking it once.
 * Ascending queries continue from the previous answer (O(prefix) total, no allocation); a smaller
 * one restarts. A byte inside a code point rounds up to the end of that code point. */
export function utf8ToUtf16Cursor(text: string): (byteOffset: number) => number {
  let byte = 0;
  let unit = 0;
  return (target) => {
    if (target < byte) byte = unit = 0;
    while (unit < text.length && byte < target) {
      const c = text.codePointAt(unit)!;
      byte += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
      unit += c > 0xffff ? 2 : 1;
    }
    return unit;
  };
}
