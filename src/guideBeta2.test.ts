import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-guide/src/templates/${page}.md`, "utf8");

it("the PDF Guide explains desktop outside-file opens and their read-only boundary (GH #732)", () => {
  for (const detail of ["![label](file:///absolute/path/paper.pdf)", "[[file:///absolute/path/paper.pdf][label]]",
    "**read-only**", "without copying", "select/copy text", "iOS and Android", "open the original link again", "256 MiB",
    "![label](../papers/paper.pdf)", "[[file:../papers/paper.pdf][label]]", "from your graph folder"]) {
    expect(guide("pdf")).toContain(detail);
  }
});

it("the PDF Guide explains citing an area image (GH #869)", () => {
  for (const detail of ["clipped image", "`((block reference))`", "**Copy ref**", "Click the image to open the PDF at that highlight", "asset placeholder"]) expect(guide("pdf")).toContain(detail);
});

it("the PDF Guide teaches touchscreen pinch and one-finger scrolling", () => {
  const pdf = guide("pdf");
  expect(pdf).toContain("pinch with two fingers");
  expect(pdf).toContain("midpoint");
  expect(pdf).toContain("one finger to scroll");
});

it("the Guide explains local image icon syntax, containment and sizing", () => {
  const text = guide("pages-links-references-search");
  for (const detail of ["tine.icon:: ![](../assets/quick-capture.png)", "#+tine.icon: [[file:../assets/quick-capture.png]]", "`icon::` accepts the same links when the page has no `tine.icon::`", "Logseq prints an `icon::` value as raw text", "Logseq shows `tine.icon::` as an ordinary property", "inside the graph's `assets/`", "no larger than 1 MiB (1,048,576 bytes)", "Larger files show no icon", "width/height attributes are ignored", "no remote image is requested", "show no icon or raw link text", "property row still shows the value as written", "without reloading"]) expect(text).toContain(detail);
});

it("the Guide teaches Chinese IME reference and command input with ASCII storage", () => {
  const references = guide("pages-links-references-search");
  for (const detail of ["`【【`", "`（（`", "`(())`", "composition finishes", "`【【name】】` ends as `[[name]]`", "`《〈`", "selected text", "ASCII"]) {
    expect(references).toContain(detail);
  }
});

it("the Guide keeps census controls and workspace creation discoverable", () => {
  expect(guide("where-things-are")).toContain("**Themes** is its own Settings category");
  expect(guide("where-things-are")).toContain("Namespaces appears only when");
  expect(guide("customize-look")).toContain("**Appearance** → **Advanced** → **Custom CSS**");
  expect(guide("keep-context-visible")).toContain("**New workspace…** from the command palette");
});

it("the Guide explains Settings search results, navigation and clearing", () => {
  const settings = guide("where-things-are");
  for (const detail of ["**theme names and descriptions**", "**plugin settings**", "Choosing a plugin setting opens that plugin's form", "English terms of two or more characters"]) expect(settings).toContain(detail);
  for (const detail of ["only matching settings", "matching words highlighted", "Choose a result", "including Advanced settings", "Clear the search", "filters commands and keys within that tab", "matching description, keyword, alias or English name"]) expect(settings).toContain(detail);
});

it("the Guide teaches phone Settings categories, Back, close and Themes (#926)", () => {
  const text = guide("where-things-are");
  for (const detail of ["**category list**", "iOS edge swipe", "close icon", "Desktop and tablets", "Search results open the category"]) expect(text).toContain(detail);
  expect(guide("customize-look")).toContain("→ **Themes** and look at **Color scheme**");
});

it("the Guide explains slash glosses, accessible carry controls and current Diagnostics tools", () => {
  const tips = guide("tips");
  for (const detail of ["English keyword followed by a Chinese gloss", "Chinese glosses are searchable", "`/表`", "`<引用`", "focus one of its controls with the keyboard", "touch devices without hover", "**Rescan graph**", "**Verify synchronized graph**"]) expect(tips).toContain(detail);
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

it("the Guide teaches the sidebar tab tree (GH #704)", () => {
  const tabs = guide("keep-context-visible");
  for (const detail of ["drag it right, under another tab, to nest it there", "hold a row still for a moment, then drag", "collapses them in the sidebar list only", "**Move out of group**", "on a touch screen, hold it still;", "Closing a tab never closes the tabs nested under it", "Pinned tabs stay at the top and never nest", "pins and groups included"]) expect(tabs).toContain(detail);
  expect(guide("where-things-are")).toContain("nest one tab under another");
});

it("the Guide teaches the Favorites touch hold (D-20261019-FT1) and its menu (GH #921)", () => {
  expect(guide("where-things-are")).toContain("hold a favorite still for a moment to open its menu (the one a right-click opens), or hold it and then drag to pick it up and move it; a finger that moves first scrolls the list");
});

it("the Guide says Quick Capture takes pasted images as graph assets (GH #835)", () => {
  const capture = guide("quick-capture");
  expect(capture).toContain("Paste an image (a screenshot) or a copied file into the box");
  expect(capture).toContain("`assets` folder");
  expect(capture).toContain("Dragging files onto the capture box is not supported yet");
});

it("the Guide teaches the Quick Capture config.edn keys (GH #832, #833)", () => {
  const capture = guide("quick-capture");
  for (const detail of [":quick-capture-options {:default-page \"Inbox\"}", ":insert-today? true", ":redirect-page? true", ":quick-capture-templates {:text \"**{time}** {text}\"}", "`{time}` is the capture time", "with no timestamp", "named `quick capture`"]) expect(capture).toContain(detail);
});
