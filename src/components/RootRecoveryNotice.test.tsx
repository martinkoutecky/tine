import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { RootRecoveryNotice } from "./RootRecoveryNotice";
import { setRootRecoveryOffers, refreshRootRecovery } from "../rootRecoveryState";
import { backend } from "../backend";
import { bumpGraphEpoch } from "../graphSession";
import { resetStore } from "../document";

afterEach(() => { setRootRecoveryOffers([]); document.body.innerHTML = ""; vi.restoreAllMocks(); });

it("offers explicit recovery and Not now, and asks again on the next open",async () => {
  const orphan = { id:"old-0123456789abcdef",old_root:"/old/notes" };
  // Native-only capability, absent from the in-memory backend.
  const api = backend();
  const original = api.orphanDrafts;
  api.orphanDrafts = vi.fn(async () => [orphan]);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <RootRecoveryNotice />,root);
  try {
    await refreshRootRecovery();
    expect(root.textContent).toContain("Unsaved edits from /old/notes");
    expect([...root.querySelectorAll("button")].map(b=>b.textContent)).toEqual(["Recover into this graph","Not now"]);
    root.querySelectorAll("button")[1].click();
    expect(root.textContent).toBe("");
    await refreshRootRecovery();
    expect(root.textContent).toContain("Unsaved edits from /old/notes");
    expect(root.textContent).not.toContain("Discard");
    bumpGraphEpoch();
    expect(root.textContent).toContain("Unsaved edits from /old/notes");
    resetStore();
    expect(root.textContent).toBe("");
  } finally { dispose(); api.orphanDrafts = original; }
});
