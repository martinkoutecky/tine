import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const scripts = path.resolve(__dirname, "../scripts");
const rule = "I-12/I-21: native CLI-forwarding journeys must establish one private session bus before starting apps; imitate scripts/e2e-multigraph.mjs via scripts/lib/e2e-session-bus.mjs";

describe("native launch forwarding environment", () => {
  it("every journey launching a second APP establishes its private bus before fixture writes", () => {
    const journeys = fs.readdirSync(scripts).filter((name) => /^e2e-.*\.mjs$/.test(name))
      .map((name) => ({ name, source: fs.readFileSync(path.join(scripts, name), "utf8") }))
      .filter(({ source }) => /spawn\(APP,/.test(source));
    expect(journeys.map(({ name }) => name)).toContain("e2e-multigraph.mjs");
    for (const { name, source } of journeys) {
      expect(source, `${rule}: ${name}`).toContain('from "./lib/e2e-session-bus.mjs"');
      const setup = source.indexOf("ensurePrivateSessionBus();");
      expect(setup, `${rule}: ${name}`).toBeGreaterThan(0);
      expect(setup, `${rule}: ${name}`).toBeLessThan(source.indexOf("fs.rmSync("));
    }
  });

  it("the suite runner uses the same private-session launcher", () => {
    const source = fs.readFileSync(path.join(scripts, "run-e2e.mjs"), "utf8");
    expect(source, rule).toContain('from "./lib/e2e-session-bus.mjs"');
    expect(source, rule).toContain("privateSessionLaunch(path.join(root, script), [], env)");
    expect(source, rule).toContain("env: session?.env ?? env");
  });
});
