import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { GraphMeta } from "../types";
import { backend } from "../backend";
import { setGraphMeta } from "../graphSession";
import { setGraphTransitioning, graphTransitioning } from "../ui";
import { setToasts, toasts } from "../toasts";
import { setRootRecoveryOffers } from "../rootRecoveryState";

const host = vi.hoisted(() => ({ bind:vi.fn(async () => {}),thaw:vi.fn() }));
vi.mock("../document", async importOriginal => ({
  ...await importOriginal<typeof import("../document")>(),
  flushAll:vi.fn(async () => true),
  bindHost:host.bind,
  tryFreezeGraphRewrite:() => host.thaw,
}));
// A stopped session cannot be flushed by graph switching. Imported conflicts
// must be shown without trying to save them as a prerequisite to rebinding.
vi.mock("../graph", () => ({ loadGraphPath:vi.fn(async () => ({kind:"aborted"})) }));
import { RootRecoveryNotice } from "./RootRecoveryNotice";

afterEach(() => {
  setGraphMeta(null); setRootRecoveryOffers([]); setGraphTransitioning(false);
  setToasts([]); document.body.innerHTML=""; vi.restoreAllMocks();
});

it("rebinds the relaunched host and clears the adopted offer without flushing the stopped session",async () => {
  const api=backend(), recover=api.recoverOrphanDrafts, discover=api.orphanDrafts;
  api.recoverOrphanDrafts=vi.fn(async () => ({error:null,reloaded:null}));
  api.orphanDrafts=vi.fn(async () => []);
  setGraphMeta({root:"/new/notes"} as GraphMeta);
  setRootRecoveryOffers([{id:"old-0123456789abcdef",old_root:"/old/notes"}]);
  const root=document.createElement("div"); document.body.append(root);
  const dispose=render(() => <RootRecoveryNotice/>,root);
  try {
    root.querySelector("button")!.click();
    await vi.waitFor(() => expect(graphTransitioning()).toBe(false));
    expect(host.bind).toHaveBeenCalledOnce();
    expect(host.bind).toHaveBeenCalledWith({refreshLoaded:true});
    expect(root.textContent).toBe("");
    expect(toasts().filter(toast => toast.kind === "error")).toEqual([]);
    expect(host.thaw).toHaveBeenCalledOnce();
  } finally { dispose(); api.recoverOrphanDrafts=recover; api.orphanDrafts=discover; }
});
