import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { emptyPage, setRaw, node, selectBlock } from "../../src/document";
import { installKeybindings } from "../../src/keybindings";
import { EditorView } from "@codemirror/view";
import { PageView } from "../../src/components/Page";
import { mainPaneRouter } from "../../src/router";
import { initParser } from "../../src/render/parse";
import { startEditing, endEdit } from "../../src/editorController";
import { setCm6EditorEnabled } from "../../src/editor/cm6Setting";
import { preloadCm6Editor } from "../../src/components/EditorGate";
import { changeCodeWrapping } from "../../src/codeDisplay";
import "../../src/styles/inter.css";
import "../../src/styles/theme.css";
import "../../src/styles/app.css";
import "../../src/styles/ls-shim.css";

await initParser();
await preloadCm6Editor();
const dto = { ...emptyPage("Code gutter", "page"), id: "pages/Code gutter.md",
  blocks: [{ id: "code", raw: "```js\nconst n = 1;\n```", collapsed: false, children: [] }] };
backend().getPage = async () => dto;
mainPaneRouter.replaceActiveRoute({ kind: "page", name: dto.name, pageKind: "page" });
render(() => <PageView />, document.getElementById("root")!);
installKeybindings();
Object.assign(window, {
  selectCode() { selectBlock("code"); },
  codeState() {
    const ta = document.querySelector<HTMLTextAreaElement>("textarea.block-editor");
    const cm = document.querySelector<HTMLElement>(".cm-content");
    const view = cm && EditorView.findFromDOM(cm);
    return { raw: node("code").raw, value: ta?.value ?? view?.state.doc.toString(),
      caret: ta?.selectionStart ?? view?.state.selection.main.head };
  },
  finishCode() { endEdit("page-navigation"); return node("code").raw; },
  showNumbered(kind: string) {
    endEdit("page-navigation");
    setRaw("code", "First line");
    setCm6EditorEnabled(kind === "cm6");
    startEditing("code", 0);
  },
  showCode(count: number, kind: string, mixed: boolean, wrap: boolean, lang = "js") {
    endEdit("page-navigation");
    changeCodeWrapping(wrap);
    const body = Array.from({ length: count }, (_, i) => `const n${i} = ${i};`).join("\n");
    const before = mixed ? "Before\n" : "";
    setRaw("code", `${before}\`\`\`${lang}\n${body}\n\`\`\`${mixed ? "\nAfter" : ""}`);
    setCm6EditorEnabled(kind === "cm6");
    if (kind !== "view") startEditing("code", before.length + 4 + lang.length);
  },
});
