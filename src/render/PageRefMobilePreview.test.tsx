import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { initParser } from "./parse";
import { LONG_PRESS_DELAY } from "./longPress";
import { contextMenu, closeContextMenu, openPageContextMenu } from "../ui";
import { ContextMenu } from "../components/ContextMenu";
import { openPage, route } from "../router";
import { readFileSync } from "node:fs";

vi.mock("../nativeChrome", async (original) => ({
  ...await original<typeof import("../nativeChrome")>(), isMobilePlatform: true,
}));
beforeAll(initParser);
afterEach(() => { vi.useRealTimers(); closeContextMenu(); document.body.replaceChildren(); });

function pointer(element: Element, type: string, x = 40) {
  element.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true,
    pointerType: "touch", pointerId: 7, isPrimary: true, clientX: x, clientY: 40 }));
}

it.each(["[[Tine]]", "#Tine"])("GH #655: holding %s keeps preview and page actions reachable through the shared menu", async (raw) => {
  vi.useFakeTimers(); openPage("Before", "page");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><AstBody raw={raw} /><ContextMenu /></>, host);
  const actionIds = () => [...document.querySelectorAll("[data-page-action-id]")].map(el => el.getAttribute("data-page-action-id"));
  try {
    // Shared builder used by desktop right-click; platform-only actions retain
    // their existing availability policy.
    openPageContextMenu(40, 40, "Tine");
    const expected = actionIds();
    expect(expected).toContain("open-sidebar");
    closeContextMenu();
    const link = host.querySelector("a")!;
    pointer(link, "pointerdown");
    await vi.advanceTimersByTimeAsync(LONG_PRESS_DELAY + 100);
    pointer(link, "pointerup");
    expect(document.querySelector(".peek-popup")).not.toBeNull();
    const actions = document.querySelector<HTMLButtonElement>('.peek-popup [aria-label="Page actions"]');
    expect(actions).not.toBeNull();
    actions!.click();
    expect(contextMenu()).toMatchObject({ kind: "page", name: "Tine", fileActions: false });
    expect(actionIds()).toEqual(expected);
    expect(route()).toMatchObject({ name: "Before" });
    expect(document.querySelector(".peek-popup")).not.toBeNull();
    // GH #705: a phone has no tab strip, so the shared menu must not offer a
    // background tab the user could never reach.
    expect(actionIds()).not.toContain("open-new-tab");
    // Tapping a menu action never dismisses the preview that owns the menu.
    document.querySelector<HTMLElement>('[data-page-action-id="open-sidebar"]')!.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch" }));
    expect(document.querySelector(".peek-popup")).not.toBeNull();
    closeContextMenu();
    pointer(document.body, "pointerdown");
    expect(document.querySelector(".peek-popup")).toBeNull();
  } finally { dispose(); }
});

it.each(["[[Tine]]", "#Tine"])("mobile hold previews %s, release stays here, outside tap dismisses, next tap navigates", async (raw) => {
  vi.useFakeTimers(); openPage("Before", "page");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <AstBody raw={raw} />, host);
  try {
    const link = host.querySelector("a")!;
    pointer(link, "pointerdown");
    await vi.advanceTimersByTimeAsync(LONG_PRESS_DELAY + 100);
    expect(document.body.querySelector(".peek-popup .ls-block")).toBeTruthy();
    expect(contextMenu()).toBeNull();
    pointer(link, "pointerup");
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(route()).toMatchObject({ name: "Before" });
    expect(document.body.querySelector(".peek-popup")).toBeTruthy();
    pointer(document.body, "pointerdown");
    expect(document.body.querySelector(".peek-popup")).toBeNull();
    pointer(link, "pointerdown"); pointer(link, "pointerup");
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    expect(route()).toMatchObject({ name: "Tine" });
  } finally { dispose(); }
});

it.each(["pointermove", "pointercancel", "pointerup"])("%s cancels the pending mobile preview", async (type) => {
  vi.useFakeTimers();
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <AstBody raw="ordinary text [[Tine]]" />, host);
  try {
    const link = host.querySelector("a")!;
    pointer(link, "pointerdown"); pointer(link, type, 100);
    await vi.advanceTimersByTimeAsync(LONG_PRESS_DELAY + 100);
    expect(document.body.querySelector(".peek-popup")).toBeNull();
  } finally { dispose(); }
});

it("the mobile Guide explains reachable navigation, preview dismissal and ordinary text selection", () => {
  const guide = readFileSync("crates/tine-core/src/templates/platforms-and-mobile.md", "utf8");
  expect(guide).toContain("Sidebar, Search, Back and Forward are in the top bar");
  expect(guide).toContain("Navigation buttons at the bottom");
  expect(guide).toContain("Long-press a page reference or tag to preview");
  expect(guide).toContain("Tap outside the preview to dismiss");
  expect(guide).toContain("**Page actions**");
  expect(guide).toContain("holding ordinary text keeps the system's text selection");
});

it("holding ordinary text leaves its native selection intact", async () => {
  vi.useFakeTimers();
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <AstBody raw="ordinary text [[Tine]]" />, host);
  try {
    const text = host.firstChild!;
    const range = document.createRange(); range.selectNodeContents(text);
    const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    const selected = selection.toString();
    pointer(host, "pointerdown");
    await vi.advanceTimersByTimeAsync(LONG_PRESS_DELAY + 100);
    expect(selection.toString()).toBe(selected);
    expect(document.body.querySelector(".peek-popup")).toBeNull();
  } finally { window.getSelection()?.removeAllRanges(); dispose(); }
});
