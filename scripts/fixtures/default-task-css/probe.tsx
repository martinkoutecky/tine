import { render } from "solid-js/web";
import { PageView } from "../../../src/components/Page";
import { backend } from "../../../src/backend";
import { managedStorageRuntime } from "../../../src/managedStorageRuntime";
import { initParser } from "../../../src/render/parse";
import { journalTitle, localDayKey } from "../../../src/journal";
import { ensureLsShimStyle } from "../../../src/lsShim";
import { ensureThemeStyle } from "../../../src/themeGallery";
import { MARKERS } from "../../../src/markers";
import "../../../src/styles/inter.css";
import "@fontsource-variable/noto-emoji/wght.css";
import "../../../src/styles/theme.css";
import "../../../src/styles/app.css";
await initParser();
ensureLsShimStyle(); ensureThemeStyle();
managedStorageRuntime.bind(1, { binding_generation: 1, authority: "direct" });
const name=journalTitle(new Date());
const targetId="39400000-0000-4000-8000-000000000001";
const blocks=MARKERS.map((marker,i)=>({id:`task-${marker.toLowerCase()}`,raw:`${marker} Task ${i+1}: plain **bold** and [[linked page]]`,marker,collapsed:false,children:[]}));
blocks.push({id:targetId,raw:`DONE Referenced task\nid:: ${targetId}`,marker:"DONE",collapsed:false,children:[]});
blocks.push({id:"completed-ref",raw:`DONE Completed containing ((${targetId}))`,marker:"DONE",collapsed:false,children:[]});
const page={name,title:name,kind:"journal" as const,pre_block:null,blocks};
backend().journalFeedPage=async()=>({pages:[page],as_of_day:localDayKey(),next_before_day:null,done:true});
backend().resolveBlocks=async()=>[{page:name,kind:"journal",blocks:[blocks[MARKERS.length]]}];
render(()=><main style="max-width:900px;margin:32px auto"><PageView />
{/* Exact nested marker shape introduced by #518, retained as a CSS boundary probe
    so this standalone CSS branch also checks integration before that patch lands. */}
<section id="nested-reference-probe" class="block-content done"><span class="block-ref"><span class="block-marker marker-done">DONE</span>{" referenced body"}</span></section>
</main>,document.getElementById("root")!);
