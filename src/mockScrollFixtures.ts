import type { BlockDto } from "./types";

// Mock pages for scroll and windowing checks, outside mock.ts's size ratchet.
type Block = (raw: string, children?: BlockDto[]) => BlockDto;

// A large synthetic page (~2000 root blocks) cycling through construct types, for
// the lazy-body virtualization harness: most blocks start as deferred raw-text
// placeholders and only parse/render on scroll. Gated behind `?big` so it never
// pollutes the normal mock screenshots (it would otherwise show up in All-Pages /
// quick-switch). Reach it with `…/?big` then quick-switch (Ctrl+K) to "Big".
export function bigPageBlocks(b: Block, n: number): BlockDto[] {
  const out: BlockDto[] = [];
  for (let i = 0; i < n; i++) {
    switch (i % 6) {
      case 0:
        out.push(b(`Paragraph **${i}** with *emphasis*, a [[ref ${i % 50}]] and a #tag${i % 20}.`));
        break;
      case 1:
        out.push(b(`## Heading ${i}`));
        break;
      case 2:
        out.push(b("```js\nfunction f" + i + "(x) {\n  return x * " + i + ";\n}\n```"));
        break;
      case 3:
        out.push(b(`| Col A | Col B |\n| --- | --- |\n| row ${i} | val ${i} |\n| row ${i + 1} | val ${i + 1} |`));
        break;
      case 4:
        out.push(b(`Display math: $$\\sum_{k=0}^{${i}} k = \\frac{${i}(${i}+1)}{2}$$`));
        break;
      default:
        out.push(
          b(`Block ${i}: a longer line of prose that wraps so the placeholder height is a realistic proxy for the rendered paragraph, with a [[link ${i % 30}]].`)
        );
        break;
    }
  }
  return out.map((block) => ({ ...block, has_id: false }));
}

// GH #810 real-wheel scroll check (`?embedscroll`, scripts/shot-gh810-wheel.mjs): sections of plain rows
// with expanded block embeds, deep enough that nested outline windows remount around them.
export function embedScrollBlocks(b: Block): { source: BlockDto[]; page: BlockDto[] } {
  const source = "81000000-0000-4000-8000-000000000810";
  const section = (label: string) => b(label, Array.from({ length: 40 }, (_, i) =>
    b(i % 13 === 5 ? `{{embed ((${source}))}}` : `${label} row ${i} plain line of text`)));
  return {
    source: [b(`Embedded root\nid:: ${source}`, Array.from({ length: 9 }, (_, i) => b(`Embedded row ${i}`)))],
    page: [section("Top one"), b("Short A"), section("Top two"), b("Short B"), section("Top three"), section("Top four")],
  };
}

// GH #848/#849 (`?manyrefs`): one hub with 127 linked references on 93 pages, with the heavy content the reports
// name (quotes, code, images, citations, long text), plus a plain neighbour page to switch tabs to.
export function manyRefsPages(b: Block): { name: string; blocks: BlockDto[] }[] {
  const image = "data:image/svg+xml;utf8," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="520"><rect width="900" height="520" fill="#789"/></svg>');
  const body = (p: number, i: number): string => {
    const needle = p % 15 === 7 ? "Foreign" : "Other";
    const tail = `in page ${p} row ${i} about [[Hub]]`;
    switch ((p + i) % 6) {
      case 0: return `> Quoted [[open-source]] ${needle} material ${tail}, long enough to wrap in a narrow pane and use the quote style.`;
      case 1: return `A very long paragraph ${tail}. ` + `${needle} words and ordinary prose wrap over several lines here. `.repeat(6);
      case 2: return `Image ${needle} ${tail}\n![figure](${image})`;
      case 3: return `Code ${needle} ${tail}\n\`\`\`js\n` + Array.from({ length: 24 }, (_, k) => `const line${k} = ${k};`).join("\n") + "\n\`\`\`";
      case 4: return `${needle} math $$\\sum_{k=0}^{${p}} k$$ and a [citation](https://example.com/${p}) ${tail}`;
      default: return `Plain ${needle} row ${tail}`;
    }
  };
  const pages = Array.from({ length: 93 }, (_, p) => ({
    name: `Refpage ${String(p).padStart(2, "0")}`,
    blocks: Array.from({ length: p < 34 ? 2 : 1 }, (_, i) => b(body(p, i), [b(`Child of ${p}/${i}`)])),
  }));
  return [{ name: "Hub", blocks: [b("Hub intro"), b("A second hub row")] }, { name: "Plain neighbour", blocks: Array.from({ length: 200 }, (_, i) => b(`Neighbour row ${i}`)) }, ...pages];
}
