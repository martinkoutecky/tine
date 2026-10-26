import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { QuickShareAll } from "./QuickShareAll";
import { invalidateBinding } from "../binding";
import type { QuickSharePrepared } from "../quickShare/commands";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); vi.restoreAllMocks(); document.body.replaceChildren(); });
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
function prepared(id: string): QuickSharePrepared {
  return { prepareId: id, shareId: id, preview: { title: id, pages: [], leftOut: [], payloadBytes: 1, envelopeBytes: 1 },
    membership: { added: [`${id} new page`], removed: ["old page"] } };
}
function mount() {
  vi.spyOn(backend(), "quickShareList").mockResolvedValue({ enabled: true, error: null, account: { login: "octo", accountId: 7 },
    shares: ["first", "second", "bad"].map(id => ({ shareId: id, state: "live", login: "octo", gistId: id,
      title: id, pages: 1, createdAt: "today", accountMatches: true })) });
  vi.spyOn(backend(), "parseQuery").mockResolvedValue({ query: {} } as never);
  vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "block", exceeded: false, groups: [{ page: "Project", kind: "page",
    blocks: ["first", "second", "bad", "no-key"].map(id => ({ id, raw: "{{tine-query @page}}", collapsed: false, children: [],
      properties: [["tine.share", `octo/${id}`]] })) }] } as never);
  const prepare = vi.spyOn(backend(), "quickSharePrepare").mockImplementation(async (_request, id) => {
    if (id === "bad") throw { kind: "notFound" };
    return prepared(id!);
  });
  const discard = vi.spyOn(backend(), "quickShareDiscard").mockResolvedValue();
  const upload = vi.spyOn(backend(), "quickShareCreate");
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <QuickShareAll />, root);
  return { root, prepare, upload, discard };
}
it("previews every share and skip reason without uploading; a failed update does not stop the next", async () => {
  const { root, prepare, upload } = mount();
  let reject!: (error: unknown) => void;
  upload.mockImplementationOnce(() => new Promise((_resolve, failure) => { reject = failure; }))
    .mockResolvedValueOnce({ shareId: "second", gistId: "second", login: "octo", copied: true });
  await vi.waitFor(() => expect(root.querySelector("button")).not.toBeNull());
  root.querySelector("button")!.click();
  await vi.waitFor(() => expect(root.querySelector<HTMLButtonElement>(".quick-share-all-confirm")?.disabled).toBe(false));
  expect(prepare).toHaveBeenCalledTimes(3);
  expect(root.textContent).toContain("+1 new / −1 gone");
  expect(root.textContent).toContain("first new page");
  expect(root.textContent).toContain("no-key");
  expect(root.textContent).toContain("key");
  expect(root.textContent).toContain("GitHub has no such share");
  expect(upload).not.toHaveBeenCalled();
  root.querySelector<HTMLButtonElement>(".quick-share-all-confirm")!.click();
  await vi.waitFor(() => expect(upload).toHaveBeenCalledTimes(1));
  await tick(); expect(upload).toHaveBeenCalledTimes(1);
  reject({ kind: "maybeUpdated" });
  await vi.waitFor(() => expect(upload).toHaveBeenCalledTimes(2));
  await vi.waitFor(() => expect(root.textContent).toContain("Re-shared. The same link"));
  expect(root.textContent).toContain("The update may have finished");
  expect(upload.mock.calls.map(args => args[0])).toEqual(["first", "second"]);
  expect(root.querySelector(".quick-share-all-confirm")).toBeNull();
});
it("closing the preview discards every unconsumed payload", async () => {
  const { root, discard, upload } = mount();
  await vi.waitFor(() => expect(root.querySelector("button")).not.toBeNull()); root.querySelector("button")!.click();
  await vi.waitFor(() => expect(root.querySelector<HTMLButtonElement>(".quick-share-all-confirm")?.disabled).toBe(false));
  root.querySelector<HTMLButtonElement>(".quick-share-all-close")!.click();
  await vi.waitFor(() => expect(discard).toHaveBeenCalledTimes(2));
  expect(upload).not.toHaveBeenCalled();
});
it("a graph switch during the first update prevents subsequent uploads", async () => {
  const { root, upload, discard } = mount();
  let finish!: (value: never) => void;
  upload.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await vi.waitFor(() => expect(root.querySelector("button")).not.toBeNull()); root.querySelector("button")!.click();
  await vi.waitFor(() => expect(root.querySelector<HTMLButtonElement>(".quick-share-all-confirm")?.disabled).toBe(false));
  root.querySelector<HTMLButtonElement>(".quick-share-all-confirm")!.click();
  await vi.waitFor(() => expect(upload).toHaveBeenCalledTimes(1));
  invalidateBinding(); finish({} as never);
  await tick(); await tick();
  expect(upload).toHaveBeenCalledTimes(1);
  expect(discard).toHaveBeenCalledWith("second");
});
