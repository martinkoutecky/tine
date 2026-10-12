// Android shows the keyboard only for a tapped focus, so a route that focuses
// search or quick capture asks the host for it (ADR 0073 routes; Martin's
// device report 2026-10-10: the search shortcut opened with no keyboard).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
vi.mock("./nativeChrome", async (original) => ({ ...(await original<typeof import("./nativeChrome")>()), platformKind: "android" }));
import { backend } from "./backend";
import { openTineLink } from "./deepLinkNavigation";
import { setSwitcherOpen } from "./ui";
import { initParser } from "./render/parse";
import { resetStore } from "./document";
import { setGraphMeta } from "./graphSession";
import { setToasts } from "./toasts";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); delete backend().tineLinks; resetStore(); setGraphMeta(null); setToasts([]); document.body.innerHTML = ""; setSwitcherOpen(false); });

function setup() {
  setGraphMeta({ root: "/fixture", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets" } as any);
  const showKeyboard = vi.fn(async () => {});
  backend().tineLinks = { identity: vi.fn(async () => ""), scanKnownGraphs: vi.fn(async () => []), take: vi.fn(async () => []),
    subscribe: vi.fn(async () => () => {}), handoff: vi.fn(async () => false), showKeyboard };
  return showKeyboard;
}

describe("route focus shows the Android keyboard", () => {
  it("asks for the keyboard once the routed input holds focus, not before", async () => {
    const showKeyboard = setup();
    const input = document.createElement("input"); document.body.append(input);
    let focusedWhenAsked = false;
    showKeyboard.mockImplementation(async () => { focusedWhenAsked = document.activeElement === input; });
    // The switcher focuses its input after a render, a few frames later.
    requestAnimationFrame(() => requestAnimationFrame(() => input.focus()));
    await openTineLink({ kind: "url", url: "tine://search" });
    expect(showKeyboard).toHaveBeenCalledTimes(1);
    expect(focusedWhenAsked).toBe(true);
  });
  it("does not raise the keyboard when nothing editable was focused", async () => {
    const showKeyboard = setup();
    await openTineLink({ kind: "url", url: "tine://search" });
    expect(showKeyboard).not.toHaveBeenCalled();
  });
  it("today, which focuses nothing, never asks", async () => {
    const showKeyboard = setup();
    const input = document.createElement("input"); document.body.append(input); input.focus();
    await openTineLink({ kind: "url", url: "tine://today" });
    expect(showKeyboard).not.toHaveBeenCalled();
  });
});
