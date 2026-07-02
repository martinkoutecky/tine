import { describe, expect, it } from "vitest";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";
import {
  rebulletedSourceByteToRawByte,
  sourceByteFromPlainTextByte,
  utf16ToUtf8ByteOffset,
  utf8ByteLength,
  utf8ByteToUtf16Offset,
} from "./spans";

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

  it("maps raw offsets into the editor-visible buffer using hidden property lines", () => {
    const raw = "alpha\nid:: 123\ncollapsed:: true\nbeta";
    expect(rawOffsetToVisibleOffset(raw, raw.indexOf("beta") + 2, isBuiltinHidden)).toBe("alpha\nbe".length);
    expect(rawOffsetToVisibleOffset(raw, raw.indexOf("123"), isBuiltinHidden)).toBe("alpha".length);

    const fenced = "```\nid:: visible\n```\nid:: hidden\nz";
    expect(rawOffsetToVisibleOffset(fenced, fenced.indexOf("visible"), isBuiltinHidden)).toBe("```\nid:: ".length);
    expect(rawOffsetToVisibleOffset(fenced, fenced.indexOf("hidden"), isBuiltinHidden)).toBe("```\nid:: visible\n```".length);
  });
});
