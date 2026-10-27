import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setGraphMeta, bumpGraphEpoch } from "../graphSession";
import { invalidateBinding } from "../binding";
import type { GraphMeta } from "../types";
import type { OwnerProtectionIssue } from "../ownerProtection";
import { OwnerProtectionNotice } from "./OwnerProtectionNotice";

const problem: OwnerProtectionIssue = { reason:"namespace", details:"owner-unsafe: directory permissions", repairable:true, instructions:"permissions", command:"chmod 700 -- '/account/.local/state/tine/owners'" };
const api = backend();
const original = { read:api.ownerProtectionStatus, retry:api.retryOwnerProtection };
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.(); dispose = undefined;
  api.ownerProtectionStatus = original.read; api.retryOwnerProtection = original.retry;
  setGraphMeta(null); invalidateBinding(); document.body.innerHTML=""; vi.restoreAllMocks();
});
function mount() {
  setGraphMeta({root:"/disposable/graph"} as GraphMeta);
  const root=document.createElement("div"); document.body.append(root);
  dispose=render(()=><OwnerProtectionNotice/>,root);
  return root;
}
const button=(root:HTMLElement,label:string)=>[...root.querySelectorAll("button")].find(b=>b.textContent===label)!;

it("keeps a non-dismissible actionable warning through display epochs, and clears only after successful repair",async()=>{
  api.ownerProtectionStatus=vi.fn(async()=>problem);
  api.retryOwnerProtection=vi.fn(async()=>null);
  const root=mount();
  await vi.waitFor(()=>expect(root.textContent).toContain("Single-writer protection and the command line are unavailable"));
  expect(root.textContent).toContain("You can keep editing");
  expect(root.textContent).toContain("private coordination folder");
  expect(root.textContent).toContain(problem.command);
  expect(root.textContent).not.toMatch(/Dismiss|Not now/);
  const details = root.querySelector("details")!;
  expect(details.open).toBe(false);
  expect(details.textContent).toContain("private coordination folder");
  expect(details.textContent).toContain(problem.command);
  expect(details.textContent).toContain(problem.details);
  expect(details.querySelector("summary")?.textContent).toBe("Details");
  details.open = true;
  bumpGraphEpoch();
  expect(root.querySelector('[role="status"]')).not.toBeNull();
  await vi.waitFor(()=>expect(api.ownerProtectionStatus).toHaveBeenCalledTimes(2));
  button(root,"Repair permissions").click();
  await vi.waitFor(()=>expect(root.textContent).toBe(""));
  expect(api.retryOwnerProtection).toHaveBeenCalledWith(true);
});

it("Retry now preserves a failed condition and diagnostic copying uses the clipboard facade",async()=>{
  api.ownerProtectionStatus=vi.fn(async()=>problem);
  api.retryOwnerProtection=vi.fn(async()=>problem);
  const copy=vi.spyOn(api,"writeText").mockResolvedValue();
  const root=mount();
  await vi.waitFor(()=>expect(root.textContent).toContain("Repair permissions"));
  button(root,"Retry now").click();
  await vi.waitFor(()=>expect(api.retryOwnerProtection).toHaveBeenCalledWith(false));
  expect(root.querySelector('[role="status"]')).not.toBeNull();
  root.querySelector("details")!.open = true;
  button(root,"Copy diagnostic details").click();
  expect(copy).toHaveBeenCalledWith(problem.details);
  button(root,"Copy command").click();
  expect(copy).toHaveBeenCalledWith(problem.command);
});

it("gives sandbox instructions instead of a repair button when the app cannot fix access",async()=>{
  api.ownerProtectionStatus=vi.fn(async():Promise<OwnerProtectionIssue>=>({...problem,repairable:false,instructions:"flatpak",command:"flatpak override --user --filesystem='/account/.local/state/tine/owners' org.example.Notes"}));
  const root=mount();
  await vi.waitFor(()=>expect(root.textContent).toContain("Grant this Flatpak access"));
  const details = root.querySelector("details")!;
  expect(details.open).toBe(false);
  expect(details.textContent).toContain("Grant this Flatpak access");
  expect(details.textContent).toContain("flatpak override");
  expect(button(root,"Repair permissions")).toBeUndefined();
  expect(button(root,"Retry now")).toBeDefined();
});

it("ignores a repair result after the graph binding changes",async()=>{
  api.ownerProtectionStatus=vi.fn(async()=>problem);
  let finish!: (result:OwnerProtectionIssue|null)=>void;
  api.retryOwnerProtection=vi.fn(()=>new Promise<OwnerProtectionIssue|null>(resolve=>{finish=resolve;}));
  const root=mount();
  await vi.waitFor(()=>expect(root.textContent).toContain("Repair permissions"));
  button(root,"Retry now").click();
  invalidateBinding(); setGraphMeta(null); bumpGraphEpoch();
  finish(problem);
  await vi.waitFor(()=>expect(root.textContent).toBe(""));
});
