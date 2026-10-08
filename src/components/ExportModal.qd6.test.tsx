import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ExportModal } from "./ExportModal";
import { initParser } from "../render/parse";
import { closeExportModal, openExportModal, openExportNodesModal } from "../ui";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { backend } from "../backend";
import { clearTransientLayersForTest } from "../transientLayers";

const { ready, uuid, nested } = vi.hoisted(() => ({ready:new Set<string>(), uuid:"12345678-1234-1234-1234-123456789abc",nested:"22345678-1234-1234-1234-123456789abc"}));
vi.mock("../resolveBatch", () => ({
  resolveBlockBatched: vi.fn(async (id:string) => {ready.add(id); return null;}),
  resolvedBlockRefSync: (id: string) => ready.has(id) ? { page: "P", blocks: [{ raw: id === uuid ? `**target** ((${nested}))` : "*nested*", children: [] }] } : null,
}));
beforeAll(initParser);
afterEach(() => { closeExportModal(); clearTransientLayersForTest(); document.body.innerHTML = ""; localStorage.clear(); resetStore(); ready.clear(); vi.restoreAllMocks(); });
it("GH #407 copies resolved Markdown, HTML and OPML through the shared modal", async () => {
  vi.spyOn(backend(), "getPage").mockResolvedValue({name:"Embed", kind:"page", blocks:[{id:"e",raw:"*embedded*",collapsed:false,children:[]}]} as any);
  const write = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <ExportModal />, root);
  const nodes = [{raw:`See ((${uuid}))\nid:: ${uuid}\nx:: xx`,children:[{raw:"{{embed [[Embed]]}}",children:[]}]}];
  setDoc({byId:{
    root:{id:"root",raw:nodes[0].raw,collapsed:false,parent:null,page:"P",children:["embed"]},
    embed:{id:"embed",raw:nodes[0].children[0].raw,collapsed:false,parent:"root",page:"P",children:[]},
    pre:{id:"pre",raw:"page-prop:: omitted",collapsed:false,parent:null,page:"P",children:[]},
  },pages:[{name:"P",kind:"page",title:"P",preBlock:"pre",roots:["root"],format:"md",readOnly:false,guide:false}],feed:["P"],loaded:true});
  const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === label)!;
  const preview = () => document.querySelector<HTMLTextAreaElement>(".export-preview")!.value;
  for (const format of ["Markdown", "HTML", "OPML"]) {
    openExportModal(["root"]);
    await vi.waitFor(() => expect(button("Copy").disabled).toBe(false));
    button(format).click();
    await vi.waitFor(() => expect(preview()).toContain("target"));
    expect(preview()).not.toContain("page-prop");
    expect(preview()).not.toContain(uuid);
    expect(preview()).not.toContain("{{embed");
    expect(preview()).toContain("embedded");
    expect(preview()).toContain("nested");
    expect(preview()).not.toContain(nested);
    if (format === "Markdown") { expect(preview()).toContain("**target**"); expect(preview()).toContain("*embedded*"); expect(preview()).toContain("x:: xx"); }
    const payload = preview(); button("Copy").click();
    await vi.waitFor(() => expect(write).toHaveBeenLastCalledWith(payload));
    await Promise.resolve();
  }
  dispose();
});

it("waits for delayed embed resolution and closing retires its preview", async () => {
  let finish!: (value:any)=>void;
  vi.spyOn(backend(), "getPage").mockReturnValue(new Promise(resolve=>{finish=resolve;}));
  const write = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <ExportModal />, root);
  openExportNodesModal([{raw:"{{embed [[Delayed]]}}",children:[]}], 1);
  const button = (label:string)=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(b=>b.textContent?.trim()===label)!;
  button("Markdown").click();
  await vi.waitFor(()=>expect(button("Resolving...").disabled).toBe(true));
  expect(write).not.toHaveBeenCalled();
  closeExportModal();
  finish({name:"Delayed",blocks:[{id:"old",raw:"old content",children:[]}]});
  await Promise.resolve(); await Promise.resolve();
  openExportNodesModal([{raw:"new content",children:[]}],1);
  await vi.waitFor(()=>expect(button("Copy")).toBeDefined());
  expect(document.querySelector<HTMLTextAreaElement>(".export-preview")!.value).toContain("new content");
  expect(document.querySelector<HTMLTextAreaElement>(".export-preview")!.value).not.toContain("old content");
  dispose();
});
