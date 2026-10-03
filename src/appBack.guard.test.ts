import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/** I-2 / GH #492, #501: the Back ladder (transient, then drawer, then router
 * history, then root) has ONE owner, src/appBack.ts `dispatchAppBack`. Android's
 * native listener and the iOS edge swipe both call it; a second copy of the
 * order is how Android once popped a route behind an open modal. Imitate
 * src/appBack.ts: add a new Back source by calling dispatchAppBack with the
 * shared deps object built in App.tsx, never by re-encoding the order. */
function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sources(p, out);
    else if (/\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

describe("single Back ladder owner", () => {
  const files = sources("src").map((p) => [p, readFileSync(p, "utf8")] as const);

  it("only appBack.ts sequences the rungs; callers go through dispatchAppBack", () => {
    const callers = files.filter(([, text]) => /\bdispatchAppBack\(/.test(text)).map(([p]) => p).sort();
    expect(callers).toEqual(["src/App.tsx", "src/androidBack.ts", "src/appBack.ts"]);
    const sequencers = files
      .filter(([, text]) => /dismissTransient\(\)[\s\S]{0,200}dismissDrawer\(\)/.test(text))
      .map(([p]) => p);
    expect(sequencers).toEqual(["src/appBack.ts"]);
  });

  it("builds the Back dependencies once, shared by Android and the edge swipe", () => {
    const app = files.find(([p]) => p === "src/App.tsx")![1];
    expect(app.match(/dismissTopTransient\("back"\)/g)).toHaveLength(1);
    expect(app.match(/dismissMobileDrawer\("back"\)/g)).toHaveLength(1);
    expect(app).toContain("...backDeps");
    expect(app).toMatch(/dispatchAppBack\(\{ \.\.\.backDeps, closeRoot\(\) \{\} \}\)/);
  });
});
