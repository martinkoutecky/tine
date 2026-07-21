import { describe, expect, it } from "vitest";
import { hiccupToHtml } from "./hiccup";

function nested(depth: number): string {
  let source = '"leaf"';
  for (let i = 0; i < depth; i++) source = `[:div ${source}]`;
  return source;
}

function withNodeCount(nodes: number): string {
  return `[:span ${Array.from({ length: nodes - 1 }, () => '"x"').join(" ")}]`;
}

describe("hiccupToHtml", () => {
  it("transcribes supported tags, sugar, attrs, nested vectors, and seq children", () => {
    expect(hiccupToHtml(
      '[:div#card.notice {:class "extra" :title "A & \\"quote\\"" "width" 3} "x < y" ([:span "nested"] " tail")]',
    )).toBe(
      '<div id="card" class="notice extra" title="A &amp; &quot;quote&quot;" width="3">x &lt; y<span>nested</span> tail</div>',
    );
  });

  it("escapes text and attribute values rather than splicing source into markup", () => {
    expect(hiccupToHtml('[:span {:title "x\\" data-breakout=\\"yes><img src=\\"x"} "<b>literal</b>"]'))
      .toBe('<span title="x&quot; data-breakout=&quot;yes&gt;&lt;img src=&quot;x">&lt;b&gt;literal&lt;/b&gt;</span>');
  });

  it("drops only unsupported attribute values", () => {
    expect(hiccupToHtml('[:span {:title [:b "no"] :width 3 :open true} "x"]'))
      .toBe('<span width="3">x</span>');
  });

  it.each([
    '[:span',
    '(fn [] "x")',
    '[:span symbol]',
    '[:span #{"set"}]',
    '[:span #thing "tagged"]',
    '[:sp@n "bad tag"]',
    '[:span#bad! "bad sugar"]',
    '[:span {"bad@attr" "x"} "bad attr"]',
    '[:span "one"] [:span "two"]',
    ':span',
  ])("rejects malformed or unsupported source as a whole: %s", (source) => {
    expect(hiccupToHtml(source)).toBeNull();
  });

  it("enforces depth while parsing: 64 is accepted and 65 falls back", () => {
    expect(hiccupToHtml(nested(64))).not.toBeNull();
    expect(hiccupToHtml(nested(65))).toBeNull();
  });

  it("enforces node count while parsing: 2048 is accepted and 2049 falls back", () => {
    expect(hiccupToHtml(withNodeCount(2048))).not.toBeNull();
    expect(hiccupToHtml(withNodeCount(2049))).toBeNull();
  });

  it("counts supported attribute entries while parsing: 2047 attrs (2048 nodes) accepted, 2048 attrs falls back", () => {
    const attrMap = (entries: number) =>
      `[:span {${Array.from({ length: entries }, (_, i) => `:a${i} ${i}`).join(" ")}} ]`;
    // The element itself is one node, so k attribute entries cost 1 + k nodes.
    expect(hiccupToHtml(attrMap(2047))).not.toBeNull();
    expect(hiccupToHtml(attrMap(2048))).toBeNull();
  });

  it("counts unsupported attribute forms while parsing before dropping the attribute", () => {
    const oversizedValue = Array.from({ length: 2049 }, () => "true").join(" ");
    expect(hiccupToHtml(`[:span {:title (${oversizedValue})} "x"]`)).toBeNull();
  });

  it("rejects source larger than 64 KiB before parsing", () => {
    expect(hiccupToHtml(`[:span "${"x".repeat(64 * 1024)}"]`)).toBeNull();
  });
});
