import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { CloneGraphDialog } from "./CloneGraphDialog";
import { backend } from "../backend";
import { invalidateBinding } from "../binding";

const { choose, load } = vi.hoisted(() => ({ choose: vi.fn(), load: vi.fn() }));
vi.mock("../graph", () => ({ chooseNewGraphParent: choose, loadGraphPath: load }));
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.replaceChildren(); vi.restoreAllMocks(); vi.resetAllMocks(); });
const submit = () => document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
function mount() {
  const close = vi.fn();
  choose.mockResolvedValue({ kind: "chosen", dir: "/parent" });
  load.mockResolvedValue({ kind: "loaded", root: "/parent/notes" });
  dispose = render(() => <CloneGraphDialog onClose={close} />, document.body);
  const input = document.querySelector<HTMLInputElement>('input[type="url"]')!;
  input.value = "https://example.invalid/me/notes.git"; input.dispatchEvent(new Event("input", { bubbles: true }));
  return close;
}

it("clones once into the selected parent and opens the returned path with automation off explained", async () => {
  const clone = vi.spyOn(backend(), "gitCloneGraph").mockResolvedValue({ kind: "cloned", root: "/parent/notes" });
  const close = mount();
  expect(document.body.textContent).toContain("Automatic commits and sync stay off");
  submit();
  await vi.waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(clone).toHaveBeenCalledWith("/parent", "https://example.invalid/me/notes.git");
  expect(load).toHaveBeenCalledWith("/parent/notes");
});

it("asks for the existing token flow on auth-required, clears the password, and retries the same destination", async () => {
  const clone = vi.spyOn(backend(), "gitCloneGraph").mockResolvedValueOnce({ kind: "auth-required" }).mockResolvedValue({ kind: "cloned", root: "/parent/notes" });
  const signIn = vi.spyOn(backend(), "gitSetupSignIn").mockResolvedValue();
  const close = mount(); submit();
  await vi.waitFor(() => expect(document.querySelector('input[type="password"]')).not.toBeNull());
  const username = document.querySelector<HTMLInputElement>('input[autocomplete="username"]')!;
  const password = document.querySelector<HTMLInputElement>('input[type="password"]')!;
  username.value = "me"; password.value = "private-token";
  submit();
  expect(password.value).toBe("");
  await vi.waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(signIn).toHaveBeenCalledWith("https://example.invalid/me/notes.git", "me", "private-token");
  expect(clone).toHaveBeenCalledTimes(2);
  expect(choose).toHaveBeenCalledOnce();
});

it("preserves a finished clone after open failure and retries only opening", async () => {
  const clone = vi.spyOn(backend(), "gitCloneGraph").mockResolvedValue({ kind: "cloned", root: "/parent/notes" });
  const close = mount();
  load.mockRejectedValueOnce(new Error("couldn't open /parent/notes"));
  submit();
  await vi.waitFor(() => expect(document.querySelector('[role="alert"]')?.textContent).toContain("/parent/notes"));
  submit();
  await vi.waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(clone).toHaveBeenCalledOnce(); expect(load).toHaveBeenCalledTimes(2);
});

it("cancellation creates nothing and a non-empty-target refusal stays visible", async () => {
  const clone = vi.spyOn(backend(), "gitCloneGraph").mockRejectedValue(new Error("The destination folder is not empty."));
  const close = mount(); choose.mockResolvedValueOnce({ kind: "aborted" }); submit();
  await vi.waitFor(() => expect(document.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false));
  expect(clone).not.toHaveBeenCalled();
  submit();
  await vi.waitFor(() => expect(document.querySelector('[role="alert"]')?.textContent).toContain("not empty"));
  expect(close).not.toHaveBeenCalled(); expect(load).not.toHaveBeenCalled();
});

it("does not clone or sign in after the graph binding changes while choosing a parent", async () => {
  const clone = vi.spyOn(backend(), "gitCloneGraph").mockResolvedValue({ kind: "cloned", root: "/parent/notes" });
  mount();
  let finish!: (choice: { kind: "chosen"; dir: string }) => void;
  choose.mockImplementation(() => new Promise<{ kind: "chosen"; dir: string }>(resolve => { finish = resolve; }));
  submit(); invalidateBinding(); finish({ kind: "chosen", dir: "/parent" });
  await vi.waitFor(() => expect(document.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false));
  expect(clone).not.toHaveBeenCalled(); expect(load).not.toHaveBeenCalled();
});
