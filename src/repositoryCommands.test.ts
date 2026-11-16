import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";

const root = fileURLToPath(new URL("../", import.meta.url));
// Construct the token so this guard can scan itself without an exception.
const agentCommand = "r" + "tk";
// Quoted executable/argv tokens and plain shell commands (including pipelines).
const invocation = new RegExp(`(["'\x60])${agentCommand}\\1|(?:^|[\\s;&|(\x60])${agentCommand}(?=\\s|$)`, "m");

it("tracked programs invoke portable commands directly", () => {
  const files = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
    .split("\0").filter(Boolean);
  const violations = files.filter(file => !/\.md$/i.test(file) && existsSync(path.join(root, file)))
    .flatMap(file => {
      const text = readFileSync(path.join(root, file), "utf8");
      return text.split("\n").flatMap((line, index) =>
        invocation.test(line) ? [`${file}:${index + 1}`] : []);
    });
  expect(violations, `Invoke the real command directly; ${agentCommand} is agent-local.`).toEqual([]);
}, 30_000);

it("detects quoted argv and plain shell invocations", () => {
  for (const source of [
    `execFileSync("${agentCommand}", ["proxy", "cc"])`,
    `subprocess.run(['${agentCommand}', 'git', 'status'])`,
    `${agentCommand} cargo test`,
    `run: ${agentCommand}`,
    `echo ok && ${agentCommand} proxy node script.mjs`,
    `const command = \`${agentCommand} git status\`;`,
  ]) expect(invocation.test(source), source).toBe(true);
  for (const source of ['execFileSync("git", ["status"])', "sortKey", `${agentCommand}_git`]) {
    expect(invocation.test(source), source).toBe(false);
  }
});
