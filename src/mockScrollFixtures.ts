// Generated fixture files for the browser mock's size and scroll flags
// (`?big`, `?manyrefs`): graph files the core reads, built before it opens.

/** One outline node: its block text and its children. */
type Node = string | [string, Node[]];

/** Markdown outline text: tab-indented bullets, continuation lines under the bullet. */
function outline(nodes: Node[], depth = 0): string {
  return nodes.map(node => {
    const [raw, children] = typeof node === "string" ? [node, []] : node;
    const indent = "\t".repeat(depth);
    const text = raw.split("\n").join(`\n${indent}  `);
    return `${indent}- ${text}\n${outline(children, depth + 1)}`;
  }).join("");
}

/** The `?big` page's block texts (~2000 root blocks) cycling through construct types, for the lazy-body
 * virtualization harness: most blocks start as deferred raw-text placeholders and only parse/render on scroll.
 * Reach it with `…/?big` then quick-switch (Ctrl+K) to "Big". */
export function bigPageRaws(n: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < n; i++) {
    switch (i % 6) {
      case 0: out.push(`Paragraph **${i}** with *emphasis*, a [[ref ${i % 50}]] and a #tag${i % 20}.`); break;
      case 1: out.push(`## Heading ${i}`); break;
      case 2: out.push("```js\nfunction f" + i + "(x) {\n  return x * " + i + ";\n}\n```"); break;
      case 3: out.push(`| Col A | Col B |\n| --- | --- |\n| row ${i} | val ${i} |\n| row ${i + 1} | val ${i + 1} |`); break;
      case 4: out.push(`Display math: $$\\sum_{k=0}^{${i}} k = \\frac{${i}(${i}+1)}{2}$$`); break;
      default:
        out.push(`Block ${i}: a longer line of prose that wraps so the placeholder height is a realistic proxy for the rendered paragraph, with a [[link ${i % 30}]].`);
    }
  }
  return out;
}

/** GH #848/#849 (`?manyrefs`): one hub with 127 linked references on 93 pages, with the heavy content the reports
 * name (quotes, code, images, citations, long text), plus a plain neighbour page to switch tabs to. */
function manyRefsFiles(): Record<string, string> {
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
  const files: Record<string, string> = {
    "pages/Hub.md": outline(["Hub intro", "A second hub row"]),
    "pages/Plain neighbour.md": outline(Array.from({ length: 200 }, (_, i) => `Neighbour row ${i}`)),
  };
  for (let p = 0; p < 93; p++) {
    files[`pages/Refpage ${String(p).padStart(2, "0")}.md`] = outline(Array.from({ length: p < 34 ? 2 : 1 },
      (_, i): Node => [body(p, i), [`Child of ${p}/${i}`]]));
  }
  return files;
}

/** The generated files a URL's flags add (`?big[&blocks=N][&long]`, `?manyrefs`). */
export function generatedFlagFiles(search: string): Record<string, string> {
  const params = new URLSearchParams(search);
  const files: Record<string, string> = {};
  if (/[?&]big\b/.test(search)) {
    const n = Math.min(5000, Math.max(1, Number(params.get("blocks")) || 2000));
    const raws = bigPageRaws(n).map((raw, i) => params.has("long")
      ? `Paragraph ${i}: ` + "Long prose with ordinary words and wrapping lines. ".repeat(80) : raw);
    files["pages/Big.md"] = "title:: Big\n\n" + outline(raws);
  }
  if (/[?&]manyrefs\b/.test(search)) Object.assign(files, manyRefsFiles());
  return files;
}
