import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("full Inter OpenType capability (GH #298)", () => {
  it("ships the audited upstream v4.1 variable fonts instead of feature-stripped subsets", () => {
    const pkg = JSON.parse(readFileSync("node_modules/inter-ui/package.json", "utf8"));
    const normal = readFileSync("node_modules/inter-ui/variable/InterVariable.woff2");
    const italic = readFileSync("node_modules/inter-ui/variable/InterVariable-Italic.woff2");
    expect(pkg.version).toBe("4.1.1");
    expect(createHash("sha256").update(normal).digest("hex"))
      .toBe("693b77d4f32ee9b8bfc995589b5fad5e99adf2832738661f5402f9978429a8e3");
    expect(createHash("sha256").update(italic).digest("hex"))
      .toBe("e564f652916db6c139570fefb9524a77c4d48f30c92928de9db19b6b5c7a262a");
    expect(readFileSync("src/main.tsx", "utf8")).toContain('import "./styles/inter.css"');
    const css = readFileSync("src/styles/inter.css", "utf8");
    expect(css).toContain('font-family: "Inter"');
    expect(css).toContain("font-weight: 100 900");
    expect(css).toContain("InterVariable.woff2");
    expect(css).toContain("InterVariable-Italic.woff2");
    expect(readFileSync("src/styles/theme.css", "utf8")).toContain('"Inter", -apple-system');
  });
});
