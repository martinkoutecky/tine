import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide makes Beta's unbound search, editing and tab commands discoverable", () => {
  const tips = guide("tips");
  for (const detail of ["**Search page names**", "**Search block content**", "**Edit selected block**", "**Activate tab 1**", "**Activate last tab**", "Ctrl/Cmd+1–9", "every included chip"]) {
    expect(tips).toContain(detail);
  }
});

it("the Guide explains compatible plugin updates and the complete Chinese interface", () => {
  const plugins = guide("plugins");
  for (const detail of ["API 0.2 and 0.3", "`color` setting", "last compatible version", "**Active ancestry only**"]) expect(plugins).toContain(detail);
  const language = guide("where-things-are");
  expect(language).toContain("Simplified Chinese covers the complete interface catalog");
  expect(language).not.toContain("Translation is under way");
});
