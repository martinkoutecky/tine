import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");

it("the 0.9 Beta Guide explains creating ordinary comment blocks from the CLI", () => {
  const cli = guide("command-line");
  for (const detail of [
    "tine add \"My comment\" --under BLOCK",
    "quote::",
    "quote-prefix::",
    "quote-suffix::",
    "tine prop set COMMENT quote \"quoted words\"",
    "--author NAME",
    "[[Features/Tips & shortcuts]]",
  ]) expect(cli).toContain(detail);
  expect(cli).toContain("On Linux, macOS and Windows");
  expect(cli).toContain("using a private named pipe for your Windows account");
  expect(cli).not.toContain("On Windows, the owner endpoint is unavailable");
});

it("the Guide names both Settings shortcuts and their remapping surface (GH #762)", () => {
  const where = guide("where-things-are");
  expect(where).toContain("**Cmd+,** on macOS or **Ctrl+,** elsewhere");
  expect(where).toContain("**t s**");
  expect(where).toContain("**Keyboard shortcuts**");
});

it("the Guide explains a collapsed reference count's stale indicator (GH #746)", () => {
  const refs = guide("pages-links-references-search");
  expect(refs).toContain("Collapsed reference sections keep their last count");
  expect(refs).toContain("**Count may be out of date**");
  expect(refs).toContain("Open the section to refresh its count and results");
});

it("the Guide distinguishes referencing blocks from pages (GH #755)", () => {
  const refs = guide("pages-links-references-search");
  expect(refs).toContain("Hover the Linked or Unlinked References count");
  expect(refs).toContain("N blocks on M pages");
  expect(refs).toContain("the count follows the active filter");
});

it("the Guide describes priority and marker page references", () => {
  const journals = guide("journals-tasks-scheduling");
  expect(journals).toContain("A header priority is also a reference to its letter's page");
  expect(journals).toContain("`[[A]]` or `[[TODO]]`");
  expect(journals).toContain("Linked References");
});

it("the Guide's tab instructions follow available width on every platform", () => {
  const context = guide("keep-context-visible");
  expect(context).toContain("below 640 px");
  expect(context).toContain("At 640 px or wider, the tab strip is available on every platform");
  expect(context).not.toContain("the tab strip and splits are off");
});

it("the Guide documents where Search-tab results open and the setting that changes it (GH #796)", () => {
  const refs = guide("pages-links-references-search");
  expect(refs).toContain("Clicking a result opens a new tab and keeps Search unchanged");
  expect(refs).toContain("**Settings → Editor → Advanced → Search results open in**");
  expect(refs).toContain("**New tab** is the default");
  expect(guide("tips")).toContain("**Settings → Editor → Advanced → Search results open in → This tab**");
});

it("the Beta Guide explains seen marks, controls and the reachable notice", () => {
  const files = guide("files-external-edits-backups");
  for (const detail of ["thin bar in the margin", "A block was removed here", "Your own commits never count", "**Mark page seen**", "**Mark whole graph seen**", "**Forget seen state**", "notice stays at the top while you scroll"]) expect(files).toContain(detail);
  for (const detail of ["Whenever there is a change, even a single one", "**Previous change**", "**Next change**", "page order", "wrapped around", "without changing the file", "never put a block into edit mode", "**Next change since you last looked**", "**Previous change since you last looked**"]) expect(files).toContain(detail);
  expect(files).not.toContain("When there is more than one change");
});

it("the Beta Guide covers git sign-in, divergence, repair and phone lifecycle", () => {
  const files = guide("files-external-edits-backups");
  for (const detail of ["**Sign in**", "**Commit and sync automatically**", "**both have new commits**", "**Copy the git command**", "**Turn off line-ending conversion for this repository**", "When you switch to another app", "at most once per interval", "never starts bringing in changes while in the background"]) expect(files).toContain(detail);
});

it("onboarding documents explicit clone and init with git optional and automation off", () => {
  const files = guide("files-external-edits-backups");
  for (const detail of ["**Clone a graph from a git URL**", "**Make this graph a git repository**", "Git is optional", "without changing any page file", "default branch", "non-empty destination", "same private file", "sync stay **Off**", "**Empty remote HTTPS URL**", "**Finish repository setup**", "A non-empty remote is refused"]) expect(files).toContain(detail);
  expect(files).not.toContain("Tine never creates or clones");
});

it("the Beta Guide makes quick-share links discoverable on phones", () => {
  const share = guide("share-pages");
  for (const detail of ["On Android and iOS", "**Link**", "**Connect GitHub**", "compatible reader"]) expect(share).toContain(detail);
});

it("the Beta Guide explains theme typography independently of colors and style", () => {
  const extend = guide("extend-tine");
  for (const detail of ["**Use typography**", "**Use colors**", "**Use style**", "**Typography**", "fonts already installed on your device", "fallback fonts"]) expect(extend).toContain(detail);
});

it("the Beta Guide covers page folding and read-only viewing folds", () => {
  for (const detail of ["Collapse all blocks on current page", "Expand all blocks on current page", "**t o**", "one Undo step"]) expect(guide("tips")).toContain(detail);
  for (const detail of ["reloading restores the original folds", "JavaScript turned off"]) expect(guide("share-pages")).toContain(detail);
});

it("the Beta Guide describes desktop selection formatting and macOS history", () => {
  expect(guide("tips")).toContain("formatting toolbar follows your selection");
  expect(guide("where-things-are")).toContain("**Cmd+[** and **Cmd+]**");
});

it("the Beta Guide explains distribution updates and existing PDF view state", () => {
  expect(guide("platforms-and-mobile")).toContain("TINE_PACKAGED_BY");
  for (const detail of ["**Open another view**", "Each view keeps its own page and zoom", "Your page and zoom are restored", "a highlight made, recolored or removed in one appears in the other at once"]) expect(guide("pdf")).toContain(detail);
});

it("the Beta Guide covers sidebar tabs and query refresh without unrelated reads", () => {
  for (const detail of ["**Open tabs**", "hold a row still", "**Move out of group**", "Pinned tabs", "come back after a restart"]) expect(guide("keep-context-visible")).toContain(detail);
  for (const detail of ["Queries refresh when an edit can affect their results", "1.5 seconds", "Continuous typing keeps the last answer visible"]) expect(guide("queries")).toContain(detail);
});

it("the Beta Guide maps a graph with journals disabled to its home page", () => {
  expect(guide("where-things-are")).toContain(":feature/enable-journals? false");
  expect(guide("where-things-are")).toContain("configured home page");
});
