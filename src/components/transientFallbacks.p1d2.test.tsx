import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { HelpPopup } from "./HelpShortcuts";
import { InPageFind } from "./InPageFind";
import { PageProps } from "./PageProps";
import { installKeybindings } from "../keybindings";
import { closeInPageFind, inPageFindActiveIndex, inPageFindOpen, inPageFindQuery, openInPageFind } from "../inpageFind";
import { closeHelpPopup, closePageProps, helpPopupOpen, openPageProps, pagePropsPanel, toggleHelpPopup } from "../ui";
import { clearTransientLayersForTest, registerTransientLayer, topTransientLayer } from "../transientLayers";
import { loadSingle, readPageProperty, resetStore } from "../store";
import { focusPane, resetPaneLayoutToSingle } from "../panes";
import type { PaneSnapshot } from "../router";
import { PAGE_PROP_SPECS } from "../editor/properties";
import { initParser } from "../render/parse";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

function mount(node: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(node, host) };
}

const pageSnapshot = (name: string): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

const journalsSnapshot = (): PaneSnapshot => ({
  tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }],
  activeIndex: 0,
});

function pagePropsInput(host: HTMLElement, key: string): HTMLInputElement {
  const spec = PAGE_PROP_SPECS.find((candidate) => candidate.key === key);
  expect(spec).toBeDefined();
  const field = Array.from(host.querySelectorAll<HTMLElement>(".pp-field"))
    .find((candidate) => candidate.querySelector(".pp-label")?.textContent === spec!.label);
  const input = field?.querySelector<HTMLInputElement>("input.pp-input");
  expect(input).not.toBeNull();
  return input!;
}

function inputValue(input: HTMLInputElement, value: string, caret: number) {
  input.focus();
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, cancelable: true }));
  input.setSelectionRange(caret, caret);
}

beforeAll(async () => {
  await initParser();
});

async function openFindAboveHelp(host: HTMLElement) {
  openInPageFind();
  await tick();
  const input = host.querySelector<HTMLInputElement>(".inpage-find-input");
  expect(input).not.toBeNull();
  input!.focus();
  input!.value = "needle";
  input!.dispatchEvent(new Event("input", { bubbles: true, cancelable: true }));
  toggleHelpPopup();
  await tick();
  expect(helpPopupOpen()).toBe(true);
  return input!;
}

async function openPagePropsAboveHelp(host: HTMLElement) {
  openPageProps("P1D2 fallback page", 20, 20);
  await tick();
  const input = host.querySelector<HTMLInputElement>(".pp-input");
  expect(input).not.toBeNull();
  input!.focus();
  input!.value = "draft value";
  input!.dispatchEvent(new Event("input", { bubbles: true, cancelable: true }));
  input!.setSelectionRange(3, 3);
  toggleHelpPopup();
  await tick();
  expect(helpPopupOpen()).toBe(true);
  return input!;
}

afterEach(() => {
  closeInPageFind({ restoreFocus: false });
  closeHelpPopup();
  closePageProps();
  clearTransientLayersForTest();
  resetStore();
  resetPaneLayoutToSingle(journalsSnapshot());
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("GH #161 P1D2-F mounted transient input fallbacks", () => {
  it("without global capture, lets real Help close before real Find and only a later Escape closes Find", async () => {
    const { host, dispose } = mount(() => <><InPageFind /><HelpPopup /></>);
    try {
      const input = await openFindAboveHelp(host);
      const first = escape();
      input.dispatchEvent(first);
      await tick();

      expect(first.defaultPrevented).toBe(true);
      expect(helpPopupOpen()).toBe(false);
      expect(inPageFindOpen()).toBe(true);
      expect(input.value).toBe("needle");
      expect(document.activeElement).toBe(input);

      const second = escape();
      input.dispatchEvent(second);
      await tick();
      expect(second.defaultPrevented).toBe(true);
      expect(inPageFindOpen()).toBe(false);
    } finally {
      dispose();
    }
  });

  it("without global capture, lets real Help close before real PageProps and only a later Escape closes PageProps", async () => {
    const { host, dispose } = mount(() => <><PageProps /><HelpPopup /></>);
    try {
      const input = await openPagePropsAboveHelp(host);
      const first = escape();
      input.dispatchEvent(first);
      await tick();

      expect(helpPopupOpen()).toBe(false);
      expect(pagePropsPanel()).not.toBeNull();
      expect(input.value).toBe("draft value");
      expect(input.selectionStart).toBe(3);
      expect(document.activeElement).toBe(input);
      expect(first.defaultPrevented).toBe(true);

      const second = escape();
      input.dispatchEvent(second);
      await tick();
      expect(second.defaultPrevented).toBe(true);
      expect(pagePropsPanel()).toBeNull();
    } finally {
      dispose();
    }
  });

  it.each(["find", "page properties"] as const)("with global capture, changes only the top rung from the real %s input", async (surface) => {
    const { host, dispose } = surface === "find"
      ? mount(() => <><InPageFind /><HelpPopup /></>)
      : mount(() => <><PageProps /><HelpPopup /></>);
    const uninstall = installKeybindings();
    try {
      const input = surface === "find" ? await openFindAboveHelp(host) : await openPagePropsAboveHelp(host);
      const event = escape();
      input.dispatchEvent(event);
      await tick();

      expect(event.defaultPrevented).toBe(true);
      expect(helpPopupOpen()).toBe(false);
      expect(surface === "find" ? inPageFindOpen() : pagePropsPanel() != null).toBe(true);
      expect(topTransientLayer()?.id).toBe(surface === "find" ? "in-page-find" : "page-properties");
    } finally {
      uninstall();
      dispose();
    }
  });

  it.each([false, true])("leaves separately dispatched composing and keyCode-229 Escape at both real inputs untouched with global capture=%s", async (withGlobal) => {
    const { host, dispose } = mount(() => <><InPageFind /><PageProps /><HelpPopup /></>);
    const uninstall = withGlobal ? installKeybindings() : undefined;
    let bubbleSentinel: ((event: KeyboardEvent) => void) | undefined;
    try {
      const find = await openFindAboveHelp(host);
      closeHelpPopup();
      await tick();
      const props = await openPagePropsAboveHelp(host);
      closeHelpPopup();
      await tick();
      const bubbleCounts = { find: 0, props: 0 };
      bubbleSentinel = (event: KeyboardEvent) => {
        if (event.target === find) bubbleCounts.find += 1;
        if (event.target === props) bubbleCounts.props += 1;
      };
      window.addEventListener("keydown", bubbleSentinel);

      for (const [name, input, expectedBubbles] of [["find", find, 1], ["props", props, 0]] as const) {
        for (const event of [escape({ composing: true }), escape({ keyCode: 229 })]) {
          input.focus();
          const before = {
            findOpen: inPageFindOpen(),
            propsOpen: pagePropsPanel(),
            top: topTransientLayer()?.id,
            value: input.value,
            caret: input.selectionStart,
            focus: document.activeElement,
          };
          const bubblesBefore = bubbleCounts[name];
          input.dispatchEvent(event);

          expect(event.defaultPrevented).toBe(false);
          expect(inPageFindOpen()).toBe(before.findOpen);
          expect(pagePropsPanel()).toBe(before.propsOpen);
          expect(topTransientLayer()?.id).toBe(before.top);
          expect(input.value).toBe(before.value);
          expect(input.selectionStart).toBe(before.caret);
          expect(document.activeElement).toBe(before.focus);
          expect(bubbleCounts[name] - bubblesBefore).toBe(expectedBubbles);
        }
      }
    } finally {
      if (bubbleSentinel) window.removeEventListener("keydown", bubbleSentinel);
      uninstall?.();
      dispose();
    }
  });

  it("observes real Find query and both next/previous directions on a routed named page", async () => {
    const pageName = "P1D2 Find neighbors";
    resetPaneLayoutToSingle(pageSnapshot(pageName));
    focusPane("main");
    loadSingle({
      name: pageName,
      kind: "page",
      title: pageName,
      pre_block: null,
      blocks: [
        { id: "needle-one", raw: "first needle", collapsed: false, children: [] },
        { id: "needle-two", raw: "second needle", collapsed: false, children: [] },
      ],
    });
    const findMount = mount(() => <InPageFind />);
    try {
      openInPageFind();
      await tick();
      const find = findMount.host.querySelector<HTMLInputElement>(".inpage-find-input")!;
      inputValue(find, "needle", 6);
      find.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(inPageFindQuery()).toBe("needle");
      expect(inPageFindActiveIndex()).toBe(1);
      expect(inPageFindOpen()).toBe(true);
      findMount.host.querySelector<HTMLButtonElement>("button[title='Previous match (Shift+Enter)']")!.click();
      expect(inPageFindActiveIndex()).toBe(0);
      find.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true, cancelable: true }));
      expect(inPageFindActiveIndex()).toBe(1);
      expect(inPageFindOpen()).toBe(true);
    } finally {
      findMount.dispose();
    }
  });

  it("persists actual PageProps Enter and blur edits, then leaves no stale owner after close, reopen, and unmount", async () => {
    const pageName = "P1D2 writable page";
    const alias = PAGE_PROP_SPECS.find((spec) => spec.key === "alias")!;
    loadSingle({
      name: pageName,
      kind: "page",
      title: pageName,
      pre_block: `${alias.key}:: before`,
      blocks: [{ id: "writable-body", raw: "body", collapsed: false, children: [] }],
    });
    const propsMount = mount(() => <PageProps />);
    try {
      openPageProps(pageName, 20, 20);
      await tick();
      const props = pagePropsInput(propsMount.host, alias.key);
      inputValue(props, "entered alias", 7);
      props.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      expect(pagePropsPanel()).toBeNull();
      expect(readPageProperty(pageName, alias.key)).toBe("entered alias");

      openPageProps(pageName, 20, 20);
      await tick();
      const reopenedRoot = propsMount.host.querySelector<HTMLElement>(".page-props-panel")!;
      const reopened = pagePropsInput(propsMount.host, alias.key);
      expect(topTransientLayer()?.id).toBe("page-properties");
      inputValue(reopened, "blur alias", 4);
      reopened.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
      expect(readPageProperty(pageName, alias.key)).toBe("blur alias");
      expect(pagePropsPanel()).not.toBeNull();

      closePageProps();
      await tick();
      expect(topTransientLayer()).toBeUndefined();
      const liveSentinel = document.createElement("button");
      document.body.append(liveSentinel);
      const disposeSentinel = registerTransientLayer({ id: "p1d2-live-sentinel", root: () => liveSentinel, dismiss: () => true });
      expect(topTransientLayer()?.id).toBe("p1d2-live-sentinel");
      // Keep the retired owner root connected so these events really reach the
      // document-capture listeners. A detached-root dispatch is a vacuous stale-
      // listener proof because it never crosses the registry's observation seam.
      document.body.append(reopenedRoot);
      expect(reopenedRoot.isConnected).toBe(true);
      reopenedRoot.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      reopenedRoot.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("p1d2-live-sentinel");
      propsMount.dispose();
      expect(reopenedRoot.isConnected).toBe(true);
      reopenedRoot.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      reopenedRoot.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      expect(topTransientLayer()?.id).toBe("p1d2-live-sentinel");
      reopenedRoot.remove();
      disposeSentinel();
    } finally {
      propsMount.dispose();
    }
  });
});
