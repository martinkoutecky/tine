import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
vi.mock("../inputCapability", async (original) => ({ ...(await original<typeof import("../inputCapability")>()), touchPrimary: () => true }));
import { ContextMenu } from "./ContextMenu";
import { closeContextMenu, openPageContextMenu } from "../ui";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { closeInPageFind, inPageFindOpen } from "../inpageFind";

beforeAll(initParser);
afterEach(() => { closeContextMenu(); closeInPageFind({ restoreFocus: false }); resetStore(); document.body.replaceChildren(); });

it("keeps Find in page reachable from the page header on a touch-first device", () => {
  loadSingle({ name: "P", kind: "page", title: "P", pre_block: null, blocks: [] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <ContextMenu />, root);
  try {
    openPageContextMenu(10, 10, "P", "page", true);
    const find = root.querySelector<HTMLButtonElement>('[data-page-action-id="find-in-page"]');
    expect(find).not.toBeNull();
    find!.click();
    expect(inPageFindOpen()).toBe(true);
    openPageContextMenu(10, 10, "P", "page", false);
    expect(root.querySelector('[data-page-action-id="find-in-page"]')).toBeNull();
  } finally { dispose(); }
});
