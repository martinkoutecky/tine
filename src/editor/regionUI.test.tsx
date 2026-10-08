import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { Block } from "../components/Block";
import { startEditing, endEdit } from "../editorController";
import { loadSingle, resetStore } from "../document/workingSet";
import { doc } from "../document/model";
import { DatePicker } from "../components/DatePicker";
import { closeDatePicker, openDatePicker, setTypographyModeSig } from "../ui";
beforeAll(() => initParser());
afterEach(() => { closeDatePicker(); endEdit("page-navigation"); resetStore(); setTypographyModeSig("render"); document.body.innerHTML=""; });
function mount(raw: string, large = false) {
  const blocks=[{id:"target",raw,collapsed:false,children:[]},...Array.from({length:large ? 1999 : 0},(_,i)=>({id:`b${i}`,raw:`prose ${i}`,collapsed:false,children:[]}))];
  loadSingle({name:"RegionUI",title:"RegionUI",kind:"page",format:"md",pre_block:null,blocks});
  startEditing("target",0);
  const root=document.createElement("div"); document.body.append(root);
  const dispose=render(()=><Block id="target" />,root);
  const ta=root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
  return {ta,dispose};
}
function type(ta: HTMLTextAreaElement,ch: string) {
  const at=ta.selectionStart;
  ta.value=ta.value.slice(0,at)+ch+ta.value.slice(ta.selectionEnd);
  ta.setSelectionRange(at+ch.length,at+ch.length);
  ta.dispatchEvent(new InputEvent("input",{bubbles:true,inputType:"insertText",data:ch}));
}
it("OG-D1 #8: typing into an empty code card keeps the closer on its own line",()=>{
  const {ta,dispose}=mount("```js\n```");
  try {type(ta,"x");expect(doc.byId.target.raw).toBe("```js\nx\n```");expect(ta.value).toBe("x");} finally {dispose();}
});
it("OG-D1 #7: on-type typography leaves the literal editing surface intact",()=>{
  setTypographyModeSig("type");
  const {ta,dispose}=mount("```sh\necho -\n```");
  try {ta.setSelectionRange(ta.value.length,ta.value.length);type(ta,">");expect(doc.byId.target.raw).toBe("```sh\necho ->\n```");} finally {dispose();}
});
it.each(["prose","code"])("OG-D1 cost: 200 %s keystrokes on a 2k-block page",(surface)=>{
  const {ta,dispose}=mount(surface === "code" ? "```js\nx\n```" : "prose",true);
  const w=window as unknown as {__tineBench?:boolean;__tineParseStats?:{calls:number;hits:number;misses:number}};
  w.__tineBench=true;w.__tineParseStats={calls:0,hits:0,misses:0};
  try {
    ta.setSelectionRange(ta.value.length,ta.value.length);
    for(let i=0;i<200;i++) type(ta,"x");
    console.info(`OG-D1 typing ${surface}: ${JSON.stringify(w.__tineParseStats)}`);
    // D1b decision 3: the existing facet/render path parses every changed raw.
    // The edit door and typography must add ZERO parses to its 200 cold parses.
    expect(w.__tineParseStats.misses).toBe(200);
  } finally {dispose();w.__tineBench=false;}
});

it("OG-D1 #6: calendar UI inserts planning outside a leading code fence",()=>{
  const raw="```\nSCHEDULED: <2026-09-29 Tue>\n```";
  const {dispose}=mount(raw);
  const root=document.createElement("div");document.body.append(root);
  openDatePicker("target","scheduled",0,0);
  const disposePicker=render(()=><DatePicker />,root);
  try {
    const today=Array.from(root.querySelectorAll<HTMLButtonElement>(".dp-btn")).find(b=>b.textContent==="Today");
    expect(today).toBeDefined();today!.click();
    [...root.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === "Done")!.click();
    expect(doc.byId.target.raw).toContain(raw);
    expect(doc.byId.target.raw.slice(raw.length)).toContain("SCHEDULED:");
  } finally {disposePicker();dispose();}
});
