import { render } from "solid-js/web";
import { backend } from "../../src/backend";
import { emptyPage, setRaw } from "../../src/document";
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
Object.assign(window, {
  showNumbered(kind: string) {
    endEdit("page-navigation");
    setRaw("code", "First line");
    setCm6EditorEnabled(kind === "cm6");
    startEditing("code", 0);
  },
  showCode(count: number, kind: string, mixed: boolean, wrap: boolean) {
    endEdit("page-navigation");
    changeCodeWrapping(wrap);
    const body = Array.from({ length: count }, (_, i) => `const n${i} = ${i};`).join("\n");
    const before = mixed ? "Before\n" : "";
    setRaw("code", `${before}\`\`\`js\n${body}\n\`\`\`${mixed ? "\nAfter" : ""}`);
    setCm6EditorEnabled(kind === "cm6");
    if (kind !== "view") startEditing("code", before.length + 6);
  },
});
