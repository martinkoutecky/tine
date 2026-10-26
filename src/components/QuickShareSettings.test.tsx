import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Suspense } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { QuickShareSettings } from "./QuickShareSettings";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { setToasts, toasts } from "../toasts";
import type { QuickShareListed, QuickShareListing } from "../quickShare/commands";
import * as definitions from "../quickShare/definitions";

// Settings → Sharing (DESIGN §0, §3.6, §3.9) over a scripted native surface.

const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 8; i++) await tick(); };
const row = (extra: Partial<QuickShareListed>): QuickShareListed => ({ shareId: "s1", state: "live", login: "octo", gistId: "g1",
  createdAt: "2026-10-17T10:00:00Z", title: "Project", pages: 3, accountMatches: true, ...extra });
let shares: QuickShareListed[];
const listing = (extra: Partial<QuickShareListing> = {}): QuickShareListing =>
  ({ enabled: true, account: { login: "octo", accountId: 7 }, shares, error: null, ...extra });

let dispose: (() => void) | undefined;
let api: ReturnType<typeof spies>;
function spies() {
  const b = backend();
  return {
    list: vi.spyOn(b, "quickShareList").mockImplementation(async () => listing()),
    copy: vi.spyOn(b, "quickShareCopyLink").mockResolvedValue(true),
    check: vi.spyOn(b, "quickShareCheck").mockResolvedValue("notFoundYet"),
    remove: vi.spyOn(b, "quickShareDelete").mockResolvedValue("deleted"),
    forget: vi.spyOn(b, "quickShareForget").mockResolvedValue(),
    find: vi.spyOn(b, "quickShareFind").mockResolvedValue([{ gistId: "f1", createdAt: "2026-10-01T00:00:00Z" }]),
    disconnect: vi.spyOn(b, "quickShareDisconnect").mockResolvedValue(),
    confirm: vi.spyOn(b, "confirm").mockResolvedValue(true),
    open: vi.spyOn(b, "openExternal").mockResolvedValue(),
    // The graph side (D-20261018-Q1), pinned on disk bytes in src/quickShare/definitions.test.ts.
    unlist: vi.spyOn(definitions, "removeShareProperty").mockResolvedValue([]),
    // Pending markers (D-20261018-QS3), pinned on disk bytes in src/components/QuickSharePending.test.tsx.
    markers: vi.spyOn(definitions, "pendingShareMarkers").mockResolvedValue({ markers: [], exceeded: false }),
    unmark: vi.spyOn(definitions, "finishSharePending").mockResolvedValue([]),
  };
}
beforeEach(() => {
  shares = [row({}), row({ shareId: "s2", state: "pending", gistId: null, title: "Draft" }),
    row({ shareId: "s3", login: "other", accountMatches: false, title: "Theirs" })];
  api = spies();
});
afterEach(() => { dispose?.(); dispose = undefined; document.body.innerHTML = ""; closeSettings(); setToasts([]); vi.restoreAllMocks(); });

async function mount() {
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <QuickShareSettings />, root);
  await settle();
  return root;
}
const share = (root: HTMLElement, id: string) => root.querySelector<HTMLElement>(`[data-share-id="${id}"]`)!;
const has = (element: HTMLElement, cls: string) => !!element.querySelector(`.${cls}`);

it("offers the development HTTPS diagnostic without a GitHub account and shows success", async () => {
  api.list.mockImplementation(async () => listing({ account: null, connectionTestAvailable: true }));
  const probe = vi.spyOn(backend(), "quickShareTestConnection").mockResolvedValue("connected");
  const root = await mount();
  const button = Array.from(root.querySelectorAll("button")).find((b) => b.textContent === "Test connection to GitHub");
  expect(button).toBeDefined();
  expect(probe).not.toHaveBeenCalled();
  button!.click(); await settle();
  expect(probe).toHaveBeenCalledTimes(1);
  expect(root.querySelector('[role="status"]')?.textContent).toContain("HTTPS connection to GitHub succeeded.");
});

it("shows a sanitized TLS diagnostic and allows another test", async () => {
  api.list.mockImplementation(async () => listing({ connectionTestAvailable: true }));
  const probe = vi.spyOn(backend(), "quickShareTestConnection").mockResolvedValue("tls");
  const root = await mount();
  const button = Array.from(root.querySelectorAll("button")).find((b) => b.textContent === "Test connection to GitHub")!;
  button.click(); await settle();
  expect(root.querySelector('[role="status"]')?.textContent).toContain("GitHub's certificate could not be verified.");
  probe.mockResolvedValue("connected");
  button.click(); await settle();
  expect(root.querySelector('[role="status"]')?.textContent).toContain("HTTPS connection to GitHub succeeded.");
});

it("hides the HTTPS diagnostic when the native build does not offer it", async () => {
  api.list.mockImplementation(async () => listing({ connectionTestAvailable: false }));
  const root = await mount();
  expect(root.textContent).not.toContain("Test connection to GitHub");
});

it("lists this device's shares with the actions each state allows", async () => {
  const root = await mount();
  expect(root.textContent).toContain("Connected to GitHub as @octo.");
  const live = share(root, "s1"), pending = share(root, "s2"), theirs = share(root, "s3");
  expect(live.textContent).toContain("Project");
  expect(live.textContent).toContain("3 pages");
  expect([has(live, "quick-share-row-copy"), has(live, "quick-share-row-delete"), has(live, "quick-share-row-check")]).toEqual([true, true, true]);
  expect([has(pending, "quick-share-row-check"), has(pending, "quick-share-row-forget"), has(pending, "quick-share-row-copy")]).toEqual([true, true, false]);
  expect(pending.textContent).toContain("Upload not confirmed");
  expect(has(theirs, "quick-share-row-delete"), "another account's share cannot be deleted from here").toBe(false);
  expect(theirs.textContent).toContain("Made with @other; connect that account to check or delete it.");
});

it("Check keeps a share that is not found yet pending, and a found share is listed live", async () => {
  const root = await mount();
  share(root, "s2").querySelector<HTMLButtonElement>(".quick-share-row-check")!.click();
  await settle();
  expect(api.check).toHaveBeenCalledWith("s2");
  expect(share(root, "s2").textContent).toContain("Not found yet.");
  api.check.mockResolvedValueOnce("live");
  shares = [row({}), row({ shareId: "s2", title: "Draft", gistId: "g2" })];
  share(root, "s2").querySelector<HTMLButtonElement>(".quick-share-row-check")!.click();
  await settle();
  expect(share(root, "s2").dataset.state).toBe("live");
  expect(has(share(root, "s2"), "quick-share-row-copy")).toBe(true);
});

it("Check on a live share identifies the snapshot on GitHub after an uncertain update", async () => {
  api.check.mockResolvedValueOnce({current: {title: "Project", createdAt: "2026-10-26T12:00:00Z", pages: ["Project", "Notes"]}});
  const root = await mount();
  const check = share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-check");
  expect(check).not.toBeNull();
  check!.click(); await settle();
  expect(api.check).toHaveBeenCalledWith("s1");
  expect(toasts().some(toast => toast.message.includes("2026-10-26T12:00:00Z") && toast.message.includes("Project, Notes"))).toBe(true);
});

it("Delete asks first, removes a deleted share, and marks one GitHub no longer has", async () => {
  const root = await mount();
  api.confirm.mockResolvedValueOnce(false);
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(api.remove, "a declined confirmation deletes nothing").not.toHaveBeenCalled();
  api.remove.mockResolvedValueOnce("gone");
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(api.remove).toHaveBeenCalledWith({ shareId: "s1" });
  expect(share(root, "s1").textContent).toContain("No longer on GitHub");
  expect(has(share(root, "s1"), "quick-share-row-forget"), "the key goes only on Forget").toBe(true);
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-forget")!.click();
  await settle();
  expect(api.forget).toHaveBeenCalledWith("s1");
});

it("a wrong-account refusal is shown and nothing else happens", async () => {
  api.remove.mockRejectedValueOnce({ kind: "wrongAccount", login: "other" });
  const root = await mount();
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(root.querySelector("[role=alert]")!.textContent).toBe("This share belongs to @other; connect that account to delete it.");
  expect(share(root, "s1")).toBeTruthy();
});

it("Find shares on GitHub lists unknown shares with Delete only", async () => {
  const root = await mount();
  root.querySelector<HTMLButtonElement>(".quick-share-find")!.click();
  await settle();
  const found = root.querySelector<HTMLElement>('[data-gist-id="f1"]')!;
  expect(found.textContent).toContain("Tine share made elsewhere (cannot be opened from here)");
  expect([...found.querySelectorAll("button")].map((b) => b.className)).toEqual(["export-btn-secondary quick-share-found-delete"]);
  found.querySelector<HTMLButtonElement>(".quick-share-found-delete")!.click();
  await settle();
  expect(api.remove).toHaveBeenCalledWith({ gistId: "f1" });
  expect(root.querySelector('[data-gist-id="f1"]')).toBeNull();
  expect(toasts().map((toast) => toast.kind)).toContain("success");
});

it("Disconnect forgets the account here and points to GitHub's Applications settings", async () => {
  const root = await mount();
  root.querySelector<HTMLButtonElement>(".quick-share-revoke")!.click();
  await settle();
  expect(api.open).toHaveBeenCalledWith("https://github.com/settings/connections/applications/Ov23liMHqsgTkxqKqFG6");
  api.list.mockImplementation(async () => listing({ account: null }));
  root.querySelector<HTMLButtonElement>(".quick-share-disconnect")!.click();
  await settle();
  expect(api.disconnect).toHaveBeenCalledTimes(1);
  expect(root.querySelector(".quick-share-settings-connect")).toBeTruthy();
  expect(root.querySelector(".quick-share-find"), "Find needs an account").toBeNull();
});

it("an unreadable shares file is reported, not shown as no shares", async () => {
  api.list.mockImplementation(async () => listing({ shares: [], error: "Tine cannot read its list of shares (quick-shares.json); nothing was changed" }));
  const root = await mount();
  expect(root.querySelector("[role=alert]")!.textContent).toContain("Tine cannot read its list of shares");
  expect(root.querySelector(".quick-share-settings-none")).toBeNull();
});

it("Settings shows the Sharing tab only where quick share is on", async () => {
  for (const enabled of [false, true]) {
    api.list.mockImplementation(async () => listing({ enabled }));
    const root = document.createElement("div"); document.body.append(root);
    const done = render(() => <Suspense><Settings /></Suspense>, root);
    openSettings();
    await settle();
    const tabs = [...root.querySelectorAll(".settings-nav-item")].map((item) => item.textContent);
    expect(tabs.includes("Sharing"), `enabled=${enabled}`).toBe(enabled);
    done(); closeSettings(); document.body.innerHTML = "";
  }
});

// D-20261018-Q1: the Exports page lists shares by their `tine.share::` property; a share that is deleted or
// forgotten here leaves that listing too. A share GitHub no longer has keeps it until Forget, like its key.
it("Delete, Forget and a found share's Delete remove the share's tine.share:: property from the graph", async () => {
  const root = await mount();
  expect(root.textContent).toContain("The Exports page lists what you shared.");
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(api.unlist.mock.calls.map(([gist]) => gist)).toEqual(["g1"]);
  api.remove.mockResolvedValueOnce("gone");
  shares = [row({ shareId: "s4", gistId: "g4" })];
  root.remove(); dispose!(); dispose = undefined;
  const next = await mount();
  share(next, "s4").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(api.unlist.mock.calls.map(([gist]) => gist), "gone from GitHub: kept until Forget").toEqual(["g1"]);
  share(next, "s4").querySelector<HTMLButtonElement>(".quick-share-row-forget")!.click();
  await settle();
  expect(api.unlist.mock.calls.at(-1)?.[0]).toBe("g4");
  next.querySelector<HTMLButtonElement>(".quick-share-find")!.click();
  await settle();
  next.querySelector<HTMLButtonElement>('[data-gist-id="f1"] .quick-share-found-delete')!.click();
  await settle();
  expect(api.unlist.mock.calls.at(-1)?.[0], "by gist identity, whatever login the graph's reference carries").toBe("f1");
});

// Review r1 #2: the graph bookkeeping of Delete / Forget / Find → Delete belongs to the graph the action started on,
// not to the Settings view: closing Settings while GitHub answers keeps it; a graph switch cancels it.
it("closing Settings while Delete, Forget or a found share's Delete runs still removes the graph property", async () => {
  const pending = <T,>() => { let finish!: (value: T) => void; const promise = new Promise<T>((r) => { finish = r; }); return { promise, finish }; };
  for (const [action, gist] of [["delete", "g1"], ["forget", "g4"], ["found", "f1"]] as const) {
    api.unlist.mockClear();
    shares = [row({}), row({ shareId: "s4", state: "pending", gistId: "g4" })];
    const root = await mount();
    let finish: (value?: unknown) => void;
    if (action === "delete") { const gate = pending<"deleted">(); api.remove.mockReturnValueOnce(gate.promise); finish = () => gate.finish("deleted"); }
    else if (action === "forget") { const gate = pending<void>(); api.forget.mockReturnValueOnce(gate.promise); finish = () => gate.finish(); }
    else {
      root.querySelector<HTMLButtonElement>(".quick-share-find")!.click(); await settle();
      const gate = pending<"deleted">(); api.remove.mockReturnValueOnce(gate.promise); finish = () => gate.finish("deleted");
    }
    const selector = action === "delete" ? '[data-share-id="s1"] .quick-share-row-delete'
      : action === "forget" ? '[data-share-id="s4"] .quick-share-row-forget' : '[data-gist-id="f1"] .quick-share-found-delete';
    root.querySelector<HTMLButtonElement>(selector)!.click();
    await settle();
    root.remove(); dispose!(); dispose = undefined;
    finish();
    await settle();
    expect(api.unlist.mock.calls.map(([id]) => id), action).toEqual([gist]);
  }
});

it("a graph switch while Delete runs cancels the graph bookkeeping", async () => {
  const { invalidateBinding } = await import("../binding");
  let finish!: (value: "deleted") => void;
  api.remove.mockReturnValueOnce(new Promise((r) => { finish = r; }));
  const root = await mount();
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  invalidateBinding();
  finish("deleted");
  await settle();
  expect(api.unlist).not.toHaveBeenCalled();
});

it("forgetting a pending share removes only its pending marker, and a failed removal is shown, not hidden", async () => {
  const root = await mount();
  share(root, "s2").querySelector<HTMLButtonElement>(".quick-share-row-forget")!.click();
  await settle();
  expect(api.forget).toHaveBeenCalledWith("s2");
  expect(api.unlist, "a pending share has no gist, so no share property").not.toHaveBeenCalled();
  expect(api.unmark, "its `pending/<record id>` marker goes (D-20261018-QS3)").toHaveBeenCalledWith("s2", null, expect.any(Function));
  api.unlist.mockRejectedValueOnce(new Error("couldn't remove tine.share:: octo/g1 from Notes"));
  share(root, "s1").querySelector<HTMLButtonElement>(".quick-share-row-delete")!.click();
  await settle();
  expect(root.querySelector("[role=alert]")!.textContent).toBe("The share is gone, but removing its tine.share:: property failed: couldn't remove tine.share:: octo/g1 from Notes");
});
