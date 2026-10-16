import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { expect, it } from "vitest";

it("gives future classless and partially styled actions a flat themed fallback while component rules win", () => {
  const css = readFileSync("src/styles/app.css", "utf8");
  const baseline = css.match(/:where\(button\)\s*\{[^}]+\}/g) ?? [];
  expect(baseline.length).toBeGreaterThan(0);
  // jsdom incorrectly prioritizes UA styles over zero-specificity :where rules
  // and does not resolve theme variables. Browser evidence checks the real rule.
  const style = document.createElement("style");
  style.textContent = baseline.join("\n")
    .replaceAll(":where(button)", "button")
    .replaceAll("var(--border-color)", "#888")
    .replaceAll("var(--bg-primary)", "#222")
    .replaceAll("var(--text-primary)", "#eee")
    + ".own-button { border: 0; background: transparent; padding: 0; }";
  document.head.append(style);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <><button>Future action</button><button class="partial">Partial action</button><button class="own-button">Existing action</button></>, host);
  try {
    for (const button of host.querySelectorAll("button:not(.own-button)")) {
      const computed = getComputedStyle(button);
      expect(computed.appearance).toBe("none");
      expect(computed.borderTopStyle).toBe("solid");
      expect(computed.backgroundColor).toBe("rgb(34, 34, 34)");
      expect(computed.color).toBe("rgb(238, 238, 238)");
    }
    expect(getComputedStyle(host.querySelector(".own-button")!).backgroundColor).toBe("rgba(0, 0, 0, 0)");
  } finally { dispose(); host.remove(); style.remove(); }
});
