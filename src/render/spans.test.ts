import { describe, expect, it } from "vitest";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";
import {
  rebulletedSourceByteToRawByte,
  sourceByteFromPlainTextByte,
  typographicPlainSpanAttrs,
  utf16ToUtf8ByteOffset,
  utf8ByteLength,
  utf8ByteToUtf16Offset,
} from "./spans";

function decodeSm(attrs: { "data-sm"?: string } | undefined): [number, number, number][] {
  return (attrs?.["data-sm"] ?? "").split(";").filter(Boolean)
    .map((p) => p.split(":").map(Number) as [number, number, number]);
}

describe("span offset helpers", () => {
  it("converts UTF-16 offsets to UTF-8 byte offsets for emoji and CJK", () => {
    const text = "a😀文";
    expect(utf16ToUtf8ByteOffset(text, 0)).toBe(0);
    expect(utf16ToUtf8ByteOffset(text, 1)).toBe(1);
    expect(utf16ToUtf8ByteOffset(text, 3)).toBe(5);
    expect(utf16ToUtf8ByteOffset(text, 4)).toBe(8);
    expect(utf8ByteToUtf16Offset(text, 0)).toBe(0);
    expect(utf8ByteToUtf16Offset(text, 1)).toBe(1);
    expect(utf8ByteToUtf16Offset(text, 5)).toBe(3);
    expect(utf8ByteToUtf16Offset(text, 8)).toBe(4);
  });

  it("maps exact plain text bytes linearly through the source span", () => {
    expect(sourceByteFromPlainTextByte([10, 14], undefined, 0, 4)).toBe(10);
    expect(sourceByteFromPlainTextByte([10, 14], undefined, 2, 4)).toBe(12);
    expect(sourceByteFromPlainTextByte([10, 14], undefined, 4, 4)).toBe(14);
    expect(sourceByteFromPlainTextByte([10, 14], undefined, 5, 4)).toBeNull();
  });

  it("maps span_map segments and snaps rendered gaps to the next source segment", () => {
    expect(sourceByteFromPlainTextByte([2, 6], [[0, 2, 1], [1, 4, 2]], 0, 3)).toBe(2);
    expect(sourceByteFromPlainTextByte([2, 6], [[0, 2, 1], [1, 4, 2]], 1, 3)).toBe(4);
    expect(sourceByteFromPlainTextByte([2, 6], [[0, 2, 1], [1, 4, 2]], 2, 3)).toBe(5);
    expect(sourceByteFromPlainTextByte([2, 6], [[0, 2, 1], [1, 4, 2]], 3, 3)).toBe(6);

    const gapped: [number, number, number][] = [[0, 10, 1], [3, 20, 2]];
    expect(sourceByteFromPlainTextByte([10, 22], gapped, 1, 5)).toBe(20);
    expect(sourceByteFromPlainTextByte([10, 22], gapped, 2, 5)).toBe(20);
    expect(sourceByteFromPlainTextByte([10, 22], gapped, 5, 5)).toBe(22);
  });

  it("rebases lsdoc re-bulleted source bytes to raw bytes with leading whitespace", () => {
    const raw = "  😀abc";
    expect(rebulletedSourceByteToRawByte(raw, 0)).toBe(0);
    expect(rebulletedSourceByteToRawByte(raw, 2)).toBe(2);
    expect(rebulletedSourceByteToRawByte(raw, 6)).toBe(6);
    expect(rebulletedSourceByteToRawByte(raw, 999)).toBe(utf8ByteLength(raw));
  });

  it("typographic plains keep exact mapping: unchanged runs map, glyphs are gaps", () => {
    // AST text "a -> b" (6 bytes) at source span [2, 8); rendered "a → b".
    // Unchanged runs: "a " (2 bytes) and " b" (2 bytes); "→" is 3 rendered bytes.
    const attrs = typographicPlainSpanAttrs("a -> b", [2, 8], undefined)!;
    expect(attrs["data-so"]).toBe("2");
    expect(attrs["data-se"]).toBe("8");
    const sm = decodeSm(attrs);
    expect(sm).toEqual([[0, 2, 2], [5, 6, 2]]);
    // Click positions on the rendered "a → b" (7 bytes): "a"(0)→2; " "(1)→3;
    // inside the glyph (2..4) snaps forward to the " b" run's source (6, i.e.
    // right after the source "->"); "b"(6)→7.
    expect(sourceByteFromPlainTextByte([2, 8], sm, 0, 7)).toBe(2);
    expect(sourceByteFromPlainTextByte([2, 8], sm, 1, 7)).toBe(3);
    expect(sourceByteFromPlainTextByte([2, 8], sm, 3, 7)).toBe(6);
    expect(sourceByteFromPlainTextByte([2, 8], sm, 6, 7)).toBe(7);
  });

  it("typographic mapping composes through an existing span_map", () => {
    // AST text "a*b--c" from escaped source "a\\*b--c" at span [2, 9):
    // inner map: "a" [0→2], "*b--c" [1→4..9). Rendered: "a*b–c".
    const inner: [number, number, number][] = [[0, 2, 1], [1, 4, 5]];
    const attrs = typographicPlainSpanAttrs("a*b--c", [2, 9], inner)!;
    const sm = decodeSm(attrs);
    // Unchanged runs "a*b" and "c" compose: [0,2,1] stays split from [1,4,2]
    // (inner boundary), "c" (rendered byte 6: after 3-byte en dash) → source 8.
    expect(sm).toEqual([[0, 2, 1], [1, 4, 2], [6, 8, 1]]);
  });

  it("typographic plain that is ONLY a glyph degrades to coarse attrs", () => {
    const attrs = typographicPlainSpanAttrs("->", [4, 6], undefined)!;
    expect(attrs["data-so"]).toBe("4");
    expect(attrs["data-se"]).toBeUndefined();
    expect(attrs["data-sm"]).toBeUndefined();
  });

  it("maps raw offsets into the editor-visible buffer using hidden property lines", () => {
    const raw = "alpha\nid:: 123\ncollapsed:: true\nbeta";
    expect(rawOffsetToVisibleOffset(raw, raw.indexOf("beta") + 2, isBuiltinHidden)).toBe("alpha\nbe".length);
    expect(rawOffsetToVisibleOffset(raw, raw.indexOf("123"), isBuiltinHidden)).toBe("alpha".length);

    const fenced = "```\nid:: visible\n```\nid:: hidden\nz";
    expect(rawOffsetToVisibleOffset(fenced, fenced.indexOf("visible"), isBuiltinHidden)).toBe("```\nid:: ".length);
    expect(rawOffsetToVisibleOffset(fenced, fenced.indexOf("hidden"), isBuiltinHidden)).toBe("```\nid:: visible\n```".length);
  });
});
