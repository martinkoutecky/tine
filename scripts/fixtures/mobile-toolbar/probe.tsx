import { For, createEffect } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "../../../src/components/Block";
import { MobileKeyboardToolbar } from "../../../src/components/MobileKeyboardToolbar";
import { focusedEditorCommandBridge } from "../../../src/editorCommandBridge";
import { installKeybindings } from "../../../src/keybindings";
import { initParser } from "../../../src/render/parse";
import { startEditing, endEdit } from "../../../src/editorController";
import { doc, loadSingle, pageByName, resetStore } from "../../../src/store";
import "../../../src/styles/inter.css";
import "../../../src/styles/theme.css";
import "../../../src/styles/app.css";
await initParser();
installKeybindings();
const trace:unknown[]=[];
const node=(id:string,children:any[]=[])=>({id,raw:id,collapsed:false,children});
let previousToolbar:Element|null=null;
const commands:string[]=[];
render(()=>{
 createEffect(()=>{
  const bridge=focusedEditorCommandBridge();trace.push({kind:"bridge",id:bridge?.blockId??null});
  if(bridge){const dispatch=bridge.dispatch;bridge.dispatch=command=>{commands.push(command);return dispatch(command)};}
 });
 return <><main class="main-content" style="padding:32px;height:500px"><For each={pageByName("Toolbar probe")?.roots??[]}>{id=><Block id={id}/>}</For></main><MobileKeyboardToolbar/></>;
},document.getElementById("root")!);
for(const type of ["pointerdown","pointerup","pointercancel","click"]) document.addEventListener(type,e=>{
 const p=e as PointerEvent;trace.push({kind:type,target:(e.target as Element).closest("button")?.getAttribute("aria-label"),pointerId:p.pointerId,pointerType:p.pointerType,detail:p.detail,x:p.clientX,y:p.clientY});
},true);
new MutationObserver(()=>{const current=document.querySelector("[data-mobile-keyboard-toolbar]");if(current!==previousToolbar){trace.push({kind:"toolbar",replaced:!!previousToolbar,present:!!current});previousToolbar=current}}).observe(document.body,{childList:true,subtree:true});
(window as any).toolbarProbe={trace,commands,doc,async setup(kind:string){
 endEdit("blur");resetStore();
 const blocks=kind==="outdent"?[node("grand",[node("parent",[node("current")])]),node("last")]:kind==="indent"?[node("parent",[node("previous")]),node("current"),node("last")]:[node("a"),node("b"),node("current"),node("d"),node("e")];
 loadSingle({name:"Toolbar probe",title:"Toolbar probe",kind:"page",pre_block:null,blocks});
 startEditing("current",3);
 await new Promise(r=>setTimeout(r,30));trace.length=0;commands.length=0;
}};
