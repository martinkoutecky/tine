import { describe, expect, it } from "vitest";
// @ts-expect-error: plain ESM build script, no types.
import { compare, identity, readBaseline, scan, scanSource } from "../../scripts/i18n-literals.mjs";

type Site = { file: string; kind: string; text: string; line: number };

// tine#107 G4 (DESIGN rev. 2 #4, #6): no NEW English reaches the UI outside the
// catalog, and nothing resolves a message at module load.
describe("the localization ratchet", () => {
  it("finds no literal UI text outside the baseline, no stale baseline entry, and no module-level t()", () => {
    const { sites, violations } = scan() as { sites: Site[]; violations: string[] };
    expect(violations).toEqual([]);
    const { added, gone } = compare(sites, readBaseline()) as { added: Site[]; gone: string[] };
    expect(added.map((site) => `${site.file}:${site.line} ${site.kind} ${site.text}`),
      "new literal UI text: put it in src/i18n/locales/en and use t()/msg()").toEqual([]);
    expect(gone, "converted sites: run node scripts/i18n-literals.mjs --update").toEqual([]);
  });

  it("counts JSX text, UI attributes, toasts, table labels and dialogs, by identity", () => {
    const { sites } = scanSource("src/components/Example.tsx", `
      import { msg, t } from "../i18n";
      import { pushToast } from "../toasts";
      const ROWS = [{ id: "a", label: "Open the thing" }, { id: "b", label: msg("command-go-search") }];
      export function Example() {
        pushToast(\`Could not save \${name}.\`, "error");
        pushToast(String(error), "error");
        pushToast(msg("toast-copied"), "success");
        if (!confirm("Delete it?")) return null;
        return <button title="Close the pane" aria-label={t("toast-dismiss")}>Close {name}</button>;
      }
    `) as { sites: Site[] };
    expect(sites.map((site) => `${site.kind} ${site.text}`)).toEqual([
      "prop:label Open the thing",
      "toast Could not save {}.",
      "toast {String(error)}",
      "dialog:confirm Delete it?",
      "attr:title Close the pane",
      "jsx-text Close",
    ]);
    // The same text twice in one file is two sites; one baseline entry covers one.
    const twice = [...sites, sites[0]];
    expect(compare(twice, sites.map(identity)).added).toHaveLength(1);
  });

  it("rejects resolving a message at module level, and allows it inside a function", () => {
    const { violations } = scanSource("src/x.ts", `
      import { t, render as show } from "./i18n";
      const LABEL = t("toast-copy");
      const OTHER = show(m);
      export const later = () => t("toast-copy");
      export class C { label = t("toast-copy"); }
    `) as { violations: string[] };
    expect(violations).toHaveLength(2);
    expect(violations[0]).toContain("src/x.ts:3");
  });
});
