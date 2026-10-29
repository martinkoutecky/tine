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
