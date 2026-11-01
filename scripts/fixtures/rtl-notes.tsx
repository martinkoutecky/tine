// GH #823: a mixed Persian, Arabic, Hebrew and English page for scripts/shot-rtl-notes.mjs. Invented text only.
import { render } from "solid-js/web";
import { For } from "solid-js";
import { initParser } from "../../src/render/parse";
import { loadSingle } from "../../src/document/workingSet";
import { pageByName } from "../../src/document/model";
import { Block, SurfaceContext } from "../../src/components/Block";
import { startEditing, endEdit } from "../../src/editorController";
import { setCm6EditorEnabled } from "../../src/editor/cm6Setting";
import { setNoteLayoutRtl } from "../../src/noteLayoutSetting";
import type { BlockDto } from "../../src/types";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";
import "../../src/styles/comments.css";

const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
await initParser();
loadSingle({ name: "RTL", title: "RTL", kind: "page", pre_block: null, format: "md", blocks: [
  leaf("fa", "این یک یادداشت فارسی است که باید از راست به چپ نوشته شود و وقتی بلند است در همان جهت به خط بعد می‌رود و تراز راست می‌ماند.", [
    leaf("fa-child", "فرزند فارسی با کلمه English در میان و عدد 42 در متن.", [
      leaf("he", "הערה בעברית, רמה שלישית של קינון.", []),
    ]),
    leaf("en-child", "An English child under a Persian parent stays left to right.", []),
  ]),
  { ...leaf("todo", "TODO [#A] انجام کار فارسی با نشانگر لاتین در آغاز منبع"), marker: "TODO", priority: "A" },
  leaf("ref", "[[Tine]] یک برنامه است و #یادداشت را هم می‌فهمد.", []),
  leaf("ar", "هذه ملاحظة عربية مع `inline code` ورابط [موقع](https://example.org) في الوسط.", []),
  leaf("en", "A plain English block with a [[صفحه فارسی]] reference and a #برچسب tag inside.", []),
  leaf("plain-en", "English words only.", []),
  leaf("mixed-rtl-end", "یادداشت فارسی English", []),
  leaf("mixed-ltr-start", "[[یادداشت]] English prose", []),
  leaf("mixed-ltr-end", "English prose یادداشت", [leaf("rtl-child", "فرزند فارسی", [])]),
  leaf("multiline", "TODO یادداشت\nEnglish یادداشت\nیادداشت English", []),
  leaf("mark", "‏English words, made right to left with a leading mark.", []),
  leaf("code", "```js\nconst a = 1; // کد همیشه چپ به راست\n```", []),
  leaf("num", "12345", []),
  leaf("empty", "", []),
  leaf("list", "فهرست:\n+ مورد اول\n+ مورد دوم\n\n> نقل قول فارسی", []),
  leaf("folded", "بلوک تاشده", [leaf("folded-child", "فرزند", [])]),
] });
Object.assign(window, { startEditing, endEdit, setCm6EditorEnabled, setNoteLayoutRtl });
render(() => <main class="main-content" style={{ padding: "24px", width: "720px" }}>
  <SurfaceContext.Provider value="main">
    <For each={pageByName("RTL")!.roots}>{id => <Block id={id} />}</For>
  </SurfaceContext.Provider>
</main>, document.getElementById("root")!);
