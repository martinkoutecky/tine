import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const source = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

function tsxSources(root: string): string[] {
  const out: string[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const dir = pending.pop()!;
    for (const entry of readdirSync(join(process.cwd(), dir), { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) pending.push(path);
      else if (entry.name.endsWith(".tsx") && !entry.name.endsWith(".test.tsx")) out.push(path);
    }
  }
  return out;
}

describe("I-22 hostile-content link shape (master b61bb9d25303)", () => {
  it("every component/render anchor with an href opens natively instead of navigating the WebView", () => {
    const offenders = [...tsxSources("src/components"), ...tsxSources("src/render")].flatMap((path) => {
      if (path === "src/components/ExternalLink.tsx") return [];
      const text = source(path);
      return [...text.matchAll(/<a\b[\s\S]{0,240}?\bhref\s*=/g)].flatMap((match) => {
        if (path === "src/components/ImproveTab.tsx" && text.slice(match.index, match.index + 300).includes("href={ISSUES_URL}")) return [];
        // A hand-written anchor must cancel the default navigation itself.
        const opening = text.slice(match.index, text.indexOf("</a>", match.index));
        return opening.includes("preventDefault()") ? [] : [`${path}@${match.index}`];
      });
    });
    expect(offenders, "I-22: graph-authored hrefs compose ExternalLink (exemplar src/components/ExternalLink.tsx) or cancel navigation and open natively").toEqual([]);
  });
});
