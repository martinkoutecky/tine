import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { QuickShareDialog, QUICK_SHARE_STATUS_POLL_MS } from "./QuickShareDialog";
import { closeQuickShare, openQuickShare, quickShareRequest } from "../shareState";
import { clearTransientLayersForTest } from "../transientLayers";
import { setToasts } from "../toasts";
import type { QueryPublicationRequest } from "../types";
import type { QuickShareConnectStart, QuickShareListing, QuickSharePrepared } from "../quickShare/commands";

// The quick share dialog (DESIGN §3.2, §3.5, §3.9) over a scripted native surface: what it shows, which commands it
// calls and how often. The native side (mocked GitHub, records, redaction) is pinned in src-tauri/src/quick_share/tests.rs.

const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 8; i++) await tick(); };
const REQUEST: QueryPublicationRequest = { argument: "@page and name = 'Project'", dialect: "macro_tql", properties: [],
  currentPage: "Project", hostBlockId: null, name: "Project", includeQueryText: false };
const ACCOUNT = { login: "octo", accountId: 7 };
const listing = (extra: Partial<QuickShareListing> = {}): QuickShareListing =>
  ({ enabled: true, account: ACCOUNT, shares: [], error: null, ...extra });
let prepared = 0;
const preparedOf = (): QuickSharePrepared => ({ prepareId: `p${++prepared}`, shareId: `Rec0rd-id_0123456789x${prepared}`, preview: { title: "Project",
  pages: [{ name: "Project", path: "pages/Project.md", journal: false }], leftOut: [{ name: "Secret", path: "pages/Secret.md", reason: "private" }],
  payloadBytes: 900, envelopeBytes: 2048 } });

let dispose: (() => void) | undefined;
let api: ReturnType<typeof spies>;
function spies() {
  const b = backend();
  return {
    list: vi.spyOn(b, "quickShareList").mockResolvedValue(listing()),
    prepare: vi.spyOn(b, "quickSharePrepare").mockImplementation(async () => preparedOf()),
    discard: vi.spyOn(b, "quickShareDiscard").mockResolvedValue(),
    create: vi.spyOn(b, "quickShareCreate").mockResolvedValue({ shareId: "s1", gistId: "g1", login: "octo", copied: true }),
    check: vi.spyOn(b, "quickShareCheck").mockResolvedValue("live"),
    copy: vi.spyOn(b, "quickShareCopyLink").mockResolvedValue(true),
    start: vi.spyOn(b, "quickShareConnectStart").mockResolvedValue({ attemptId: "a1", userCode: "WDJB-MJHT", verificationUri: "https://github.com/login/device", expiresIn: 900 }),
    status: vi.spyOn(b, "quickShareConnectStatus").mockResolvedValue({ state: "pending" }),
    cancel: vi.spyOn(b, "quickShareConnectCancel").mockResolvedValue(),
    open: vi.spyOn(b, "openExternal").mockResolvedValue(),
  };
}
beforeEach(() => { prepared = 0; api = spies(); });
afterEach(() => {
  dispose?.(); dispose = undefined; document.body.innerHTML = "";
  closeQuickShare(); clearTransientLayersForTest(); setToasts([]); vi.useRealTimers(); vi.restoreAllMocks();
});

async function open() {
  const root = document.createElement("div"); document.body.append(root);
  openQuickShare(REQUEST);
  dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
  await settle();
  return root;
}
const button = (root: HTMLElement, cls: string) => root.querySelector<HTMLButtonElement>(`.${cls}`);
const text = (root: HTMLElement) => root.textContent ?? "";

it("prepares the reviewed request and shows what goes out before anything is uploaded", async () => {
  const root = await open();
  expect(api.prepare).toHaveBeenCalledTimes(1);
  expect(api.prepare.mock.calls[0][0]).toEqual(REQUEST);
  expect(text(root)).toContain("pages/Project.md");
  expect(text(root), "what is left out, and why").toContain("Secret");
  expect(text(root)).toContain("The full text of these pages is shared, including their properties.");
  expect(text(root)).toContain("Shares the pages as shown; refresh to include later edits.");
  expect(text(root), "the size of what GitHub stores").toContain("2.0 KB");
  expect(api.create, "nothing is uploaded without the click").not.toHaveBeenCalled();
});

it("creates exactly once from the prepared payload and says the link was copied, without the key in the page", async () => {
  const root = await open();
  button(root, "quick-share-create")!.click();
  button(root, "quick-share-create")?.click(); // a second click while uploading
  await settle();
  expect(api.create).toHaveBeenCalledTimes(1);
  expect(api.create).toHaveBeenCalledWith("p1");
  expect(text(root)).toContain("Link copied.");
  expect(text(root), "a masked link: login and gist, never the key").toContain("https://tine.page/s/1/#octo/g1/…");
  expect(button(root, "quick-share-create"), "a share is made once; Create is gone").toBeNull();
  button(root, "quick-share-copy")!.click();
  await settle();
  expect(api.copy).toHaveBeenCalledWith("s1");
  dispose!(); dispose = undefined;
  expect(api.discard, "a consumed payload needs no discard").not.toHaveBeenCalled();
});

it("keeps the dialog open with the cause on a failure, and needs a fresh preview to try again", async () => {
  api.create.mockRejectedValueOnce({ kind: "network" });
  const root = await open();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(root.querySelector(".quick-share-dialog")).toBeTruthy();
  expect(root.querySelector("[role=alert]")!.textContent).toContain("Tine could not reach GitHub, so nothing was sent.");
  expect(button(root, "quick-share-create")!.disabled, "the consumed payload is not uploaded again").toBe(true);
  button(root, "quick-share-refresh")!.click();
  await settle();
  expect(api.prepare).toHaveBeenCalledTimes(2);
  button(root, "quick-share-create")!.click();
  await settle();
  expect(api.create.mock.calls.map((call) => call[0])).toEqual(["p1", "p2"]);
});

it("maps each failure to a sentence naming its cause", async () => {
  const cases: [unknown, string][] = [
    [{ kind: "reconnect" }, "Reconnect GitHub"],
    [{ kind: "rateLimited", minutes: 3 }, "GitHub is limiting requests; try again in 3 minutes."],
    [{ kind: "github", message: "Spammy content" }, "GitHub refused: Spammy content"],
    [{ kind: "unavailable" }, "GitHub did not answer as expected."],
    [{ kind: "previewExpired" }, "This preview expired."],
    [{ kind: "refused", message: "at most 200 pages" }, "Tine cannot share this: at most 200 pages"],
  ];
  for (const [failure, sentence] of cases) {
    api.create.mockRejectedValueOnce(failure);
    const root = await open();
    button(root, "quick-share-create")!.click();
    await settle();
    expect(root.querySelector("[role=alert]")?.textContent, JSON.stringify(failure)).toContain(sentence);
    dispose!(); dispose = undefined; closeQuickShare(); document.body.innerHTML = "";
  }
});

it("an upload that may have finished offers Check, and a found share offers Copy link", async () => {
  api.create.mockRejectedValueOnce({ kind: "maybeUploaded", shareId: "s9" });
  api.check.mockResolvedValueOnce("notFoundYet").mockResolvedValueOnce("live");
  const root = await open();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(root.querySelector("[role=alert]")!.textContent).toBe("The upload may have finished. Check");
  expect(button(root, "quick-share-create"), "no second upload of a maybe-uploaded share").toBeNull();
  button(root, "quick-share-check")!.click();
  await settle();
  expect(text(root)).toContain("Not found yet.");
  button(root, "quick-share-check")!.click();
  await settle();
  expect(api.check.mock.calls.map((call) => call[0])).toEqual(["s9", "s9"]);
  expect(text(root)).toContain("Found on GitHub: the share is live.");
  button(root, "quick-share-copy")!.click();
  await settle();
  expect(api.copy).toHaveBeenCalledWith("s9");
  expect(api.create).toHaveBeenCalledTimes(1);
});

it("closing the dialog discards the prepared payload", async () => {
  await open();
  closeQuickShare();
  await settle();
  expect(api.discard).toHaveBeenCalledWith("p1");
});

it("first use connects GitHub: shows the code and opens github.com/login/device, then prepares", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout"] });
  api.list.mockResolvedValueOnce(listing({ account: null })).mockResolvedValue(listing());
  api.status.mockResolvedValueOnce({ state: "pending" }).mockResolvedValueOnce({ state: "connected", login: "octo" });
  const root = await openWithTimers();
  expect(api.prepare, "nothing is prepared before an account exists").not.toHaveBeenCalled();
  expect(text(root)).toContain("Tine asks for GitHub's gist permission.");
  expect(text(root)).toContain("No GitHub account? On GitHub's sign-up page choose Continue with Google.");
  button(root, "quick-share-connect-start")!.click();
  await vi.advanceTimersByTimeAsync(0);
  expect(root.querySelector(".quick-share-code")!.textContent).toBe("WDJB-MJHT");
  expect(api.open).toHaveBeenCalledWith("https://github.com/login/device");
  await vi.advanceTimersByTimeAsync(QUICK_SHARE_STATUS_POLL_MS);
  expect(text(root)).toContain("Waiting for you to approve Tine on GitHub");
  await vi.advanceTimersByTimeAsync(QUICK_SHARE_STATUS_POLL_MS);
  await vi.advanceTimersByTimeAsync(0);
  expect(api.status.mock.calls.every((call) => call[0] === "a1")).toBe(true);
  expect(api.prepare).toHaveBeenCalledTimes(1);
  expect(text(root)).toContain("Connected to GitHub as @octo.");
});

it("a missing gist permission fails the connection with GitHub's reason, and closing mid-flow cancels the attempt", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout"] });
  api.list.mockResolvedValue(listing({ account: null }));
  api.status.mockResolvedValueOnce({ state: "failed", reason: "scope" });
  let root = await openWithTimers();
  button(root, "quick-share-connect-start")!.click();
  await vi.advanceTimersByTimeAsync(QUICK_SHARE_STATUS_POLL_MS);
  expect(text(root)).toContain("GitHub did not grant the gist permission");
  expect(button(root, "quick-share-connect-start")!.textContent).toBe("Try again");
  dispose!(); dispose = undefined; closeQuickShare(); document.body.innerHTML = "";
  expect(api.cancel, "an ended attempt needs no cancel").not.toHaveBeenCalled();

  api.status.mockResolvedValue({ state: "pending" });
  root = await openWithTimers();
  button(root, "quick-share-connect-start")!.click();
  await vi.advanceTimersByTimeAsync(QUICK_SHARE_STATUS_POLL_MS);
  closeQuickShare();
  await vi.advanceTimersByTimeAsync(0);
  expect(api.cancel).toHaveBeenCalledWith("a1");
  const polls = api.status.mock.calls.length;
  await vi.advanceTimersByTimeAsync(QUICK_SHARE_STATUS_POLL_MS * 3);
  expect(api.status.mock.calls.length, "polling stops with the dialog").toBe(polls);
});

it("closing the dialog while GitHub's device code is still on its way cancels that attempt when it arrives", async () => {
  api.list.mockResolvedValue(listing({ account: null }));
  let arrive!: (value: QuickShareConnectStart) => void;
  api.start.mockImplementation(() => new Promise((resolve) => { arrive = resolve; }));
  const root = await open();
  button(root, "quick-share-connect-start")!.click();
  expect(api.start).toHaveBeenCalledTimes(1);
  closeQuickShare();
  await settle();
  arrive({ attemptId: "late", userCode: "WDJB-MJHT", verificationUri: "https://github.com/login/device", expiresIn: 900 });
  await settle();
  expect(api.cancel, "Rust would otherwise keep polling GitHub for a closed view").toHaveBeenCalledWith("late");
  expect(api.open, "a closed dialog opens no browser").not.toHaveBeenCalled();
  expect(api.status).not.toHaveBeenCalled();
});

it("a revoked token sends the user back to Connect GitHub", async () => {
  api.create.mockRejectedValueOnce({ kind: "reconnect" });
  const root = await open();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(button(root, "quick-share-connect-start")).toBeTruthy();
  expect(text(root)).toContain("GitHub no longer accepts Tine's access. Reconnect GitHub.");
});

it("in a build without quick share it says so and prepares nothing", async () => {
  api.list.mockResolvedValue(listing({ enabled: false, account: null }));
  const root = await open();
  expect(root.querySelector("[role=alert]")!.textContent).toBe("Quick share is not available in this build yet.");
  expect(api.prepare).not.toHaveBeenCalled();
  expect(button(root, "quick-share-connect-start")).toBeNull();
});

async function openWithTimers() {
  const root = document.createElement("div"); document.body.append(root);
  openQuickShare(REQUEST);
  dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
  await vi.advanceTimersByTimeAsync(0);
  return root;
}

// D-20261018-Q1, D-20261018-QS3: the query block is marked `tine.share:: pending/<record id>` before the upload, and a
// created share replaces the marker with `<login>/<gist_id>` (no key), through the document layer's normal edit path
// (src/quickShare/definitions.ts, pinned there and in QuickSharePending.test.tsx on disk bytes).
it("marks the query block before the upload, then records the created share as login/gist, never with the key", async () => {
  const definitions = await import("../quickShare/definitions");
  const order: string[] = [];
  const mark = vi.spyOn(definitions, "markSharePending").mockImplementation(async () => { order.push("mark"); return "Notes"; });
  const finish = vi.spyOn(definitions, "finishSharePending").mockResolvedValue(["Notes"]);
  api.create.mockImplementationOnce(async () => { order.push("create"); return { shareId: "s1", gistId: "g1", login: "octo", copied: true }; });
  const root = document.createElement("div"); document.body.append(root);
  openQuickShare(REQUEST, { hostBlockId: "b7", macro: "{{tine-query @page and name = 'Project'}}" });
  dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
  await settle();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(order).toEqual(["mark", "create"]);
  expect(mark).toHaveBeenCalledWith({ hostBlockId: "b7", macro: "{{tine-query @page and name = 'Project'}}" }, "Rec0rd-id_0123456789x1", expect.any(Function));
  expect(finish).toHaveBeenCalledWith("Rec0rd-id_0123456789x1", "octo/g1", expect.any(Function), ["Notes"]);
  expect(text(root)).toContain("Listed on Notes (tine.share::, without the key).");
});

it("a share found by Check finishes its marker from its record's gist; a failed recording names the cause and keeps the share", async () => {
  const definitions = await import("../quickShare/definitions");
  vi.spyOn(definitions, "markSharePending").mockResolvedValue("Exports");
  const finish = vi.spyOn(definitions, "finishSharePending").mockRejectedValue(new Error("couldn't write tine.share:: octo/g9 on Exports"));
  api.create.mockRejectedValueOnce({ kind: "maybeUploaded", shareId: "Rec0rd-id_0123456789x1" });
  const root = document.createElement("div"); document.body.append(root);
  openQuickShare(REQUEST, { hostBlockId: null, macro: "{{tine-query @page and name = 'Project'}}" });
  dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
  await settle();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(finish, "an unconfirmed upload keeps its marker").not.toHaveBeenCalled();
  api.list.mockResolvedValue(listing({ shares: [{ shareId: "Rec0rd-id_0123456789x1", state: "live", login: "octo", gistId: "g9", createdAt: "2026-10-18T00:00:00Z", title: "Project", pages: 1, accountMatches: true }] }));
  button(root, "quick-share-check")!.click();
  await settle();
  expect(finish).toHaveBeenCalledWith("Rec0rd-id_0123456789x1", "octo/g9", expect.any(Function), ["Exports"]);
  expect(root.querySelector("[role=alert]")!.textContent).toContain("The share was created, but recording it in the graph failed: couldn't write tine.share:: octo/g9 on Exports");
  expect(button(root, "quick-share-copy"), "the share itself stays usable").toBeTruthy();
});

it("a definite failure removes the marker; a marker that could not be saved uploads nothing", async () => {
  const definitions = await import("../quickShare/definitions");
  const mark = vi.spyOn(definitions, "markSharePending").mockResolvedValue("Notes");
  const finish = vi.spyOn(definitions, "finishSharePending").mockResolvedValue(["Notes"]);
  api.create.mockRejectedValueOnce({ kind: "network" });
  const root = document.createElement("div"); document.body.append(root);
  openQuickShare(REQUEST, { hostBlockId: "b7", macro: "{{query (task TODO)}}" });
  dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
  await settle();
  button(root, "quick-share-create")!.click();
  await settle();
  expect(finish).toHaveBeenCalledWith("Rec0rd-id_0123456789x1", null, expect.any(Function), ["Notes"]);
  button(root, "quick-share-refresh")!.click();
  await settle();
  mark.mockRejectedValueOnce(new Error("couldn't save Notes"));
  button(root, "quick-share-create")!.click();
  await settle();
  expect(api.create, "no upload without the marker").toHaveBeenCalledTimes(1);
  expect(root.querySelector("[role=alert]")!.textContent).toBe("Tine could not mark the query block before the upload, so nothing was sent: couldn't save Notes");
  expect(button(root, "quick-share-create")!.disabled, "the preview stays for another try").toBe(false);
});

// Review r1 #2: the share exists once Create answers, so its graph record survives closing the dialog meanwhile; it
// stays bound to the graph Create started on, so a graph switch cancels it.
it("closing the dialog while Create uploads still records the share; a graph switch cancels the record", async () => {
  const definitions = await import("../quickShare/definitions");
  const { invalidateBinding } = await import("../binding");
  vi.spyOn(definitions, "markSharePending").mockResolvedValue("Exports");
  const save = vi.spyOn(definitions, "finishSharePending").mockResolvedValue(["Exports"]);
  for (const leave of ["close", "switch"] as const) {
    save.mockClear();
    let finish!: (value: { shareId: string; gistId: string; login: string; copied: boolean }) => void;
    api.create.mockReturnValueOnce(new Promise((r) => { finish = r; }));
    const root = document.createElement("div"); document.body.append(root);
    openQuickShare(REQUEST, { hostBlockId: null, macro: "{{tine-query @page and name = 'Project'}}" });
    dispose = render(() => <QuickShareDialog request={quickShareRequest} />, root);
    await settle();
    button(root, "quick-share-create")!.click();
    await settle();
    if (leave === "close") closeQuickShare(); else invalidateBinding();
    await settle();
    finish({ shareId: "s1", gistId: "g1", login: "octo", copied: true });
    await settle();
    if (leave === "close") {
      expect(save, leave).toHaveBeenCalledWith(expect.any(String), "octo/g1", expect.any(Function), ["Exports"]);
      expect((save.mock.calls[0][2] as () => boolean)(), "bound to the graph Create started on").toBe(true);
    } else expect(save, leave).not.toHaveBeenCalled();
    dispose(); dispose = undefined; root.remove(); closeQuickShare();
  }
});
