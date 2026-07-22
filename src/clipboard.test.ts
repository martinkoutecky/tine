import { afterEach, describe, it, expect, vi } from "vitest";
import { backend } from "./backend";
import {
  clearClipboardPayload,
  consumeClipboardCutGrant,
  copyBlockOutline,
  normalizeClipboardText,
  outlineToHtml,
  peekClipboardPayload,
  writeClipboardImage,
  writeClipboardRich,
  writeClipboardText,
  writeClipboardTextStrict,
  type ClipboardPayloadData,
} from "./clipboard";

const payload: ClipboardPayloadData = {
  blocks: [{ raw: "raw\nid:: 11111111-1111-1111-1111-111111111111", sourceFormat: "md", children: [] }],
  sourcePages: [{ name: "Page", kind: "page", path: "pages/page.md", generation: 7 }],
};

afterEach(() => {
  clearClipboardPayload();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("outlineToHtml (text/html clipboard flavor)", () => {
  it("builds nested <ul><li> from a tab-indented outline", () => {
    const md = "- a\n\t- b\n\t- c\n- d";
    expect(outlineToHtml(md)).toBe(
      "<ul><li>a<ul><li>b</li><li>c</li></ul></li><li>d</li></ul>"
    );
  });

  it("escapes HTML and folds continuation lines into the same <li>", () => {
    const md = "- a <x> & b\n  more";
    expect(outlineToHtml(md)).toBe("<ul><li>a &lt;x&gt; &amp; b<br>more</li></ul>");
  });

  it("empty input → empty string", () => {
    expect(outlineToHtml("")).toBe("");
  });
});

describe("private clipboard slot + facade", () => {
  it("clears old state before starting a block write, then publishes a fresh generation", async () => {
    const observations: Array<ReturnType<typeof peekClipboardPayload>> = [];
    vi.spyOn(backend(), "writeRich").mockImplementation(async () => { observations.push(peekClipboardPayload()); });

    await copyBlockOutline("copy", "- first", payload);
    const first = peekClipboardPayload();
    await copyBlockOutline("cut", "- second", payload);
    const second = peekClipboardPayload();

    expect(observations).toEqual([null, null]);
    expect(first?.op).toBe("copy");
    expect(second?.op).toBe("cut");
    expect(second!.generation).toBeGreaterThan(first!.generation);
  });

  it("all ordinary text/rich/image facade writers synchronously clear the slot", () => {
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    vi.spyOn(backend(), "writeText").mockResolvedValue();
    vi.spyOn(backend(), "copyImageToClipboard").mockResolvedValue();

    void copyBlockOutline("copy", "- block", payload);
    void writeClipboardText("export");
    expect(peekClipboardPayload()).toBeNull();
    void copyBlockOutline("copy", "- block", payload);
    void writeClipboardRich("sheet", "<table></table>");
    expect(peekClipboardPayload()).toBeNull();
    void copyBlockOutline("copy", "- block", payload);
    void writeClipboardImage(new Uint8Array([1, 2, 3]));
    expect(peekClipboardPayload()).toBeNull();
  });

  it("propagates strict text transport failure after synchronously clearing the slot", async () => {
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    await copyBlockOutline("cut", "- block", payload);
    const failure = new Error("clipboard denied");
    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(failure) } });

    await expect(writeClipboardTextStrict("report")).rejects.toBe(failure);
    expect(peekClipboardPayload()).toBeNull();
  });

  it("consumes a generation-tagged cut grant up front and downgrades the slot", () => {
    vi.spyOn(backend(), "writeRich").mockResolvedValue();
    void copyBlockOutline("cut", "- cut", payload);
    const generation = peekClipboardPayload()!.generation;

    expect(consumeClipboardCutGrant(generation + 1)).toBeNull();
    expect(consumeClipboardCutGrant(generation)).toEqual({ generation, sourcePages: payload.sourcePages });
    expect(peekClipboardPayload()?.op).toBe("copy");
    expect(consumeClipboardCutGrant(generation)).toBeNull();
  });

  it("normalizes CRLF and strips exactly one trailing newline", () => {
    expect(normalizeClipboardText("a\r\nb\r\n")).toBe("a\nb");
    expect(normalizeClipboardText("a\n\n")).toBe("a\n");
    expect(normalizeClipboardText("a")).toBe("a");
  });
});
