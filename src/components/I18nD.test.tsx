import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { ContextMenu } from "./ContextMenu";
import { CarryActions } from "./Page";
import { closeSettings, openSettings, closeContextMenu, openPageContextMenu } from "../ui";
import { resetStore, type FeedPage } from "../document";
import { setDoc } from "../document/model";
import { journalTitle, localDayKey, setCurrentDayKeyForTest } from "../journal";
import { msg, renderEnglish, t } from "../i18n";

const locale = vi.hoisted(() => ({ set: (_value: boolean) => {} }));
// Decorate catalog output to exercise live consumption before translations ship.
vi.mock("../i18n", async (original) => {
  const actual = await original<typeof import("../i18n")>();
  const { createSignal } = await import("solid-js");
  const [decorated, setDecorated] = createSignal(false);
  locale.set = setDecorated;
  const display = (text: string) => decorated() ? `«${text}»` : text;
  return {
    ...actual,
    t: (...args: Parameters<typeof actual.t>) => display(actual.t(...args)),
    render: (text: Parameters<typeof actual.render>[0]) => display(actual.render(text)),
  };
});

afterEach(() => {
  locale.set(false);
  closeSettings();
  closeContextMenu();
  setCurrentDayKeyForTest(localDayKey());
  resetStore();
  document.body.replaceChildren();
});

it("an open Settings dialog updates tab labels, controls and inline prose", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("appearance");
  await new Promise((resolve) => setTimeout(resolve, 0));
  try {
    const tabs = () => [...root.querySelectorAll(".settings-nav-item")].map((node) => node.textContent);
    expect(tabs()).toContain("Appearance");
    const prose = () => [...root.querySelectorAll(".settings-hint")].find((node) => node.textContent?.startsWith("Keep the normal"));
    expect(prose()?.textContent).toBe("Keep the normal Enter = new block and Shift + Enter = line break mapping while Document mode is on. Off (the default) swaps them, like Logseq. Saved to :shortcut/doc-mode-enter-for-new-block? in config.edn.");
    expect(prose()?.querySelector("code")?.textContent).toBe(":shortcut/doc-mode-enter-for-new-block?");
    locale.set(true);
    expect(tabs()).toContain("«Appearance»");
    expect(root.querySelector(".settings-pane-head > span")?.textContent).toBe("«Appearance»");
    expect(root.querySelector(".settings-search-input")?.getAttribute("placeholder")).toBe("«Search settings…»");
    expect(root.textContent).toContain("«Mode»");
    expect(root.textContent).toContain("«Keep the normal Enter = new block");
    expect([...root.querySelectorAll("code")].some((node) => node.textContent === ":shortcut/doc-mode-enter-for-new-block?")).toBe(true);
  } finally { dispose(); }
});

it("an open page menu resolves stored labels after a language switch and retains the page name", () => {
  const name = "Authored 中文";
  setDoc({ byId: {}, pages: [{ name, kind: "page", title: name, preBlock: null, roots: [], format: "md", readOnly: false, guide: false }], feed: [name], loaded: true });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <ContextMenu />, root);
  openPageContextMenu(10, 10, { name, pageKind: "page" }, true);
  try {
    expect(root.querySelector('[role="menu"]')?.getAttribute("aria-label")).toBe("Page actions");
    expect(root.textContent).toContain("Open in new tab");
    locale.set(true);
    expect(root.querySelector('[role="menu"]')?.getAttribute("aria-label")).toBe("«Page actions»");
    expect(root.textContent).toContain("«Open in new tab»");
    expect(root.textContent).toContain("«Copy page ref»");
    expect(renderEnglish(msg("context-menu-ui-deleted", { name }))).toBe(`Deleted “${name}”`);
  } finally { dispose(); }
});

it("mounted journal carry controls update without translating the journal title", () => {
  const name = journalTitle(new Date());
  setCurrentDayKeyForTest(localDayKey(new Date(new Date().getFullYear(), new Date().getMonth(), new Date().getDate() - 1)));
  const page: FeedPage = { name, kind: "journal", title: name, preBlock: null, roots: [], format: "md", readOnly: false, guide: false };
  const root = document.createElement("div");
  const dispose = render(() => <CarryActions page={page} />, root);
  try {
    expect(root.textContent).toContain("Carry from previous day");
    locale.set(true);
    expect(root.textContent).toContain("«Carry from previous day»");
    expect(root.querySelector("button")?.title).toBe("«Pull unfinished tasks from the most recent day that has content»");
    expect(page.title).toBe(name);
  } finally { dispose(); }
});

it("plural messages and native confirmations preserve English text and line breaks", () => {
  expect(t("context-menu-ui-copy", { count: 1 })).toBe("Copy block");
  expect(t("context-menu-ui-copy", { count: 3 })).toBe("Copy blocks");
  expect(t("settings-asset-trash-emptied", { count: 1 })).toBe("Emptied asset trash (1 file)");
  expect(t("settings-asset-trash-emptied", { count: 3 })).toBe("Emptied asset trash (3 files)");
  expect(t("settings-asset-trash-confirmation", { count: 1, size: "4 KB" })).toBe("Permanently delete 1 asset file (4 KB) in the trash?\n\nThis cannot be undone. Page, journal, and conflict recovery files in logseq/.tine-trash will be kept.");
  expect(t("settings-asset-trash-confirmation", { count: 3, size: "4 KB" })).toContain("3 asset files (4 KB)");
  expect(t("settings-trash-protected-summary", { pages: 1, journals: 2, conflicts: 3, other: 0, beforeJournals: 1, beforeConflicts: 1, beforeOther: 1 })).toBe("Protected recovery trash kept: 1 page, 2 journals, 3 conflicts");
  expect(t("settings-trash-protected-summary", { pages: 0, journals: 1, conflicts: 0, other: 2, beforeJournals: 0, beforeConflicts: 1, beforeOther: 1 })).toBe("Protected recovery trash kept: 1 journal, 2 other");
  expect(t("context-menu-child-view", { view: "outline", active: "yes" })).toBe("✓ Outline");
  expect(t("settings-ui-move-the-journal-file-to-the-trash-it-s", { name: "day.org" })).toBe("Move the journal file “day.org” to the trash?\n\nIt's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).");
  expect(t("context-menu-ui-delete-the-file-moves-to-the-graph-s-tine", { name: "A", unsaved: "discard" })).toBe('Delete "A"? The file moves to the graph\'s .tine-trash folder. Your unsaved changes to it are discarded too.');
});
