import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import baseline from "../crates/tine-core/tests/fixtures/structural-recognizers.json";

const rule = "I-12: ask lsdoc through the block-region door; no regex over content structure. Exemplar: crates/tine-core/src/block_regions.rs";
function production(source: string, rust: boolean): string {
  // rustfmt gives cfg(test) items an unindented closing brace. Tests are exempt,
  // including inline tests in production Rust modules.
  return rust ? source.replace(/^#\[cfg\(test\)\][\s\S]*?^}\s*$/gm, "") : source;
}
function count(source: string, rust: boolean): number {
  return production(source,rust).split("\n").filter(line => {
    const s = line.toLowerCase().replaceAll("\\", "");
    return ["id::",":id:","scheduled","deadline","closed",":logbook:","clock:",":properties:",":end:","```","~~~","#+begin_"].some(t => s.includes(t))
      && /starts_with|contains\(|strip_prefix|eq_ignore_ascii_case|startswith|includes\(|\.test\(|\.exec\(|regexp|regex|=\s*\/|=\s*r#?"/.test(s);
  }).length;
}
function scan(dir: string, relative = ""): Record<string,number> {
  const result:Record<string,number>={};
  for (const entry of readdirSync(dir,{withFileTypes:true})) {
    const file=join(relative,entry.name);
    if (entry.isDirectory()) {
      if (["target","node_modules","tests","examples","fixtures","wasm"].includes(entry.name)) continue;
      Object.assign(result,scan(join(dir,entry.name),file));
    } else if (/\.(rs|tsx?|js)$/.test(file) && !/\.test\.|_tests\.rs$|testSetup/.test(file) && file!=="crates/tine-core/src/block_regions.rs") {
      const n=count(readFileSync(join(dir,entry.name),"utf8"),file.endsWith(".rs"));
      if (n) result[file]=n;
    }
  }
  return result;
}

describe(rule, () => {
  it("ratchets native and frontend recognizers outside the door", () => {
    const current={...scan("src","src"),...scan("crates","crates")};
    for (const [file,n] of Object.entries(current)) expect(n,`${rule}; ${file}: ${n}`).toBeLessThanOrEqual((baseline as Record<string,number>)[file]??0);
  });
  it("detects both prohibited shapes", () => {
    expect(count('if line.starts_with("id::") { rewrite(); }',true)).toBe(1);
    expect(count('const bad = /^SCHEDULED:/m;',false)).toBe(1);
    expect(count('raw.startsWith("#+BEGIN_SRC")',false)).toBe(1);
  });
});
