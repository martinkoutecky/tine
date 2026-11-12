#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Check workflow text in O(bytes). Custom secrets are conservatively treated as
 * release credentials; the built-in token qualifies when it can publish content
 * or packages. Returns line-numbered diagnostics, without reading credentials. */
export function privilegedActionPinProblems(text) {
  const source = text.replace(/^[ \t]*#.*$/gm, "");
  const customSecret = /\bsecrets\s*(?:\.\s*(?!GITHUB_TOKEN\b)\w+|\[)/.test(source);
  const privileged = customSecret || /\bsecrets:\s*inherit\b/.test(source)
    || /["']?id-token["']?\s*:\s*["']?write\b/.test(source)
    || /["']?(?:contents|packages)["']?\s*:\s*["']?write\b/.test(source)
    || /\bpermissions:\s*["']?write-all\b/.test(source);
  if (!privileged) return [];
  const problems = [];
  for (const [index, line] of source.split("\n").entries()) {
    for (const match of line.matchAll(/(?:^|[{,])\s*(?:-\s*)?["']?uses["']?\s*:\s*["']?([^\s"'#,}]*)/g)) {
      // Fail closed: a value continued on later lines (plain or block scalar) is not checked by line.
      if (!match[1] || /^[>|]/.test(match[1])) {
        problems.push(`line ${index + 1}: uses needs its action on the same line`);
        continue;
      }
      if (match[1].startsWith("./")) continue;
      if (!/@[0-9a-f]{40}$/.test(match[1]) || !/\s+#\s+\S+/.test(line)) {
        problems.push(`line ${index + 1}: ${match[1]} needs a full commit SHA and release comment`);
      }
    }
  }
  return problems;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const directory = ".github/workflows";
  const problems = fs.readdirSync(directory).filter(name => /\.ya?ml$/.test(name)).sort()
    .flatMap(name => privilegedActionPinProblems(fs.readFileSync(path.join(directory, name), "utf8"))
      .map(problem => `${directory}/${name}: ${problem}`));
  if (problems.length) {
    console.error(problems.join("\n"));
    process.exitCode = 1;
  } else {
    console.log("Privileged workflow action pins OK");
  }
}
