import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

export function networkClaimViolations(file: string, source: string): string[] {
  const calls = /\b(?:fetch\s*\(|XMLHttpRequest\b|WebSocket\b|sendBeacon\s*\()/g;
  return [...source.matchAll(calls)].map((match) => `${file}: network API ${match[0]}`);
}

function assertNoNetwork(file: string, source: string): void {
  const found = networkClaimViolations(file, source);
  if (found.length) throw new Error(`I-11: the Improve panel and lsdoc diff make no network calls; exemplar src-tauri/src/commands.rs improve-panel claim.\n${found.join("\n")}`);
}

describe("I-11 no-network claim", () => {
  it("keeps the Improve panel and its diff helpers local", () => {
    const files = ["src/components/ImproveTab.tsx"];
    const dir = "src/devtools/lsdoc-diff";
    files.push(...readdirSync(dir).filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts")).map((name) => path.join(dir, name)));
    for (const file of files) assertNoNetwork(file, readFileSync(file, "utf8"));
  });

  it("fails a planted network call", () => {
    expect(() => assertNoNetwork("src/planted.ts", 'fetch("https://example.invalid", { body: title });'))
      .toThrow(/I-11:.*exemplar src-tauri\/src\/commands\.rs/s);
  });
});
