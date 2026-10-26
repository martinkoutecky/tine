import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the Guide teaches Chinese IME reference and command input with ASCII storage", () => {
  const references = guide("pages-links-references-search");
  for (const detail of ["`【【`", "`（（`", "`(())`", "composition finishes", "`《〈`", "selected text", "ASCII"]) {
    expect(references).toContain(detail);
  }
});

it("the Guide keeps census controls and workspace creation discoverable", () => {
  expect(guide("where-things-are")).toContain("**Themes** is the first section");
  expect(guide("where-things-are")).toContain("Namespaces appears only when");
  expect(guide("customize-look")).toContain("**Appearance** → **Advanced** → **Custom CSS**");
  expect(guide("keep-context-visible")).toContain("**New workspace…** from the command palette");
});

it("the Guide explains Settings search results, navigation and clearing", () => {
  const settings = guide("where-things-are");
  for (const detail of ["only matching settings", "matching words highlighted", "Choose a result", "including Advanced settings", "Clear the search", "filters commands and keys within that tab"]) expect(settings).toContain(detail);
});

it("the Guide explains slash glosses, accessible carry controls and current Diagnostics tools", () => {
  const tips = guide("tips");
  for (const detail of ["English keyword followed by a Chinese gloss", "Chinese glosses are searchable", "focus one of its controls with the keyboard", "touch devices without hover", "**Rescan graph**", "**Verify synchronized graph**"]) expect(tips).toContain(detail);
  expect(guide("capture-plan-day")).toContain("same controls appear on a journal page");
  expect(guide("journals-tasks-scheduling")).toContain("focus a title-row control with the keyboard");
  expect(guide("troubleshooting-recovery")).not.toContain("Run comparison");
  expect(tips).not.toContain("Help improve Tine's parser");
});

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
