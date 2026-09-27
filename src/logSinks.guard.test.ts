import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sources(file);
    return /\.tsx?$/.test(file) && !/\.test\.tsx?$/.test(file) ? [file] : [];
  });
}

export function logSinkViolations(file: string, source: string): string[] {
  const found: string[] = [];
  for (const match of source.matchAll(/console\.(?:log|warn|error|info|debug)\s*\(/g)) {
    const start = match.index + match[0].length;
    let depth = 1;
    let quote: string | null = null;
    let escaped = false;
    let end = start;
    for (; end < source.length; end++) {
      const ch = source[end];
      if (quote) {
        if (escaped) escaped = false;
        else if (ch === "\\") escaped = true;
        else if (ch === quote) quote = null;
      } else if (ch === '"' || ch === "'" || ch === "`") quote = ch;
      else if (ch === "(") depth++;
      else if (ch === ")" && --depth === 0) break;
    }
    const argument = source.slice(start, end).trim();
    if (!/^(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')$/.test(argument)) {
      const line = source.slice(0, match.index).split("\n").length;
      found.push(`${file}:${line}: console payload is not fixed text`);
    }
  }
  return found;
}

function privateFlowNetworkViolations(file: string, source: string): string[] {
  return /\b(?:fetch\s*\(|XMLHttpRequest\b|WebSocket\b|sendBeacon\s*\()/.test(source)
    ? [`${file}: network API in graph save/persistence path`] : [];
}

function assertNoPrivateNetwork(file: string, source: string): void {
  const found = privateFlowNetworkViolations(file, source);
  if (found.length) throw new Error(`I-5: graph content, titles and paths stay off network payloads; exemplar src/backend.ts native savePages IPC.\n${found.join("\n")}`);
}

function assertClean(file: string, source: string): void {
  const found = logSinkViolations(file, source);
  if (found.length) throw new Error(`I-5: WebView console calls use fixed text without titles, paths or content; exemplar src/document/save/engine.ts read-only refusal.\n${found.join("\n")}`);
}

describe("I-5 console sink scan", () => {
  it("keeps production console calls fixed", () => {
    const root = process.cwd();
    for (const absolute of sources(path.join(root, "src"))) {
      assertClean(path.relative(root, absolute), readFileSync(absolute, "utf8"));
    }
  });

  it("keeps graph save and persistence data out of network payloads", () => {
    for (const file of [...sources("src/document"), "src/backend.ts", "src/carry.ts", "src/graph.ts"]) {
      assertNoPrivateNetwork(file, readFileSync(file, "utf8"));
    }
  });

  it("fails a planted page-name log", () => {
    expect(() => assertClean("src/planted.ts", 'console.warn("failed", pageName);'))
      .toThrow(/I-5:.*exemplar src\/document\/save\/engine\.ts/s);
    expect(() => assertClean("src/planted.ts", 'console.warn("failed",\n  pageName);'))
      .toThrow(/I-5:.*exemplar src\/document\/save\/engine\.ts/s);
  });

  it("fails a planted network payload", () => {
    expect(() => assertNoPrivateNetwork("src/planted.ts", 'fetch("/collect", { body: title });'))
      .toThrow(/I-5:.*exemplar src\/backend\.ts/s);
  });
});
