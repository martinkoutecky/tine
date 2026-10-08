// RULE: package.json "scripts" has each key once, and every `node scripts/X`
// entry points at a script that exists. A duplicate key is silently last-wins
// in JSON.parse, so a copy-pasted entry could point at a stale script without
// any tool complaining. Exemplar: this file.
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const raw = readFileSync(new URL("../package.json", import.meta.url), "utf8");

function scriptsSection(): string {
  const start = raw.indexOf('"scripts"');
  const open = raw.indexOf("{", start);
  const close = raw.indexOf("\n  }", open);
  return raw.slice(open, close);
}

describe("package.json scripts", () => {
  it("has no duplicate script keys (remove the copy, keep the one whose script exists)", () => {
    const keys = [...scriptsSection().matchAll(/^\s{4}"([^"]+)"\s*:/gm)].map((m) => m[1]);
    const dups = keys.filter((k, i) => keys.indexOf(k) !== i);
    expect(dups, `duplicate package.json script keys: ${dups.join(", ")}`).toEqual([]);
  });

  it("every `node scripts/...` entry names an existing file", () => {
    const scripts: Record<string, string> = JSON.parse(raw).scripts;
    const missing: string[] = [];
    for (const [name, command] of Object.entries(scripts)) {
      for (const m of command.matchAll(/\bnode\s+(scripts\/[^\s;&|]+)/g)) {
        if (!existsSync(new URL(`../${m[1]}`, import.meta.url))) missing.push(`${name} -> ${m[1]}`);
      }
    }
    expect(missing, `package.json scripts pointing at missing files: ${missing.join("; ")}`).toEqual([]);
  });
});
