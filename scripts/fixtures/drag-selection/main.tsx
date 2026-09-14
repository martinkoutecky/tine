import { render } from "solid-js/web";
import { Block } from "../../../src/components/Block";
import { initParser } from "../../../src/render/parse";
import { loadSingle } from "../../../src/store";
import { managedStorageRuntime } from "../../../src/managedStorageRuntime";
import "../../../src/styles/app.css";
import "../../../src/styles/inter.css";
import "@fontsource-variable/noto-emoji/wght.css";

await initParser();
managedStorageRuntime.bind(1, { binding_generation: 1, authority: "direct" });
const params = new URLSearchParams(location.search);
const sentence = "This is a long multiline block containing several ordinary words that wrap naturally over many lines. ";
const shape = params.get("shape");
const raw = shape === "hard"
  ? Array.from({ length: 8 }, () => "Some ordinary text on this line.").join("\n")
  : shape === "unicode"
    ? "Café příliš žluťoučký office affinity 👩🏽‍💻 alpha العربية beta several more words. ".repeat(10)
    : shape === "code"
      ? "```js\n" + Array.from({ length: 8 }, (_, i) => `const line${i} = 'a long code line that must not wrap even in a narrow editor';`).join("\n") + "\n```"
      : sentence.repeat(8);
document.getElementById("root")!.style.width = `${params.get("width") ?? 420}px`;
document.documentElement.style.zoom = params.get("zoom") ?? "1";
loadSingle({ name: "Drag", title: "Drag", kind: "page", pre_block: null,
  blocks: [{ id: "drag-block", raw, collapsed: false, children: [] }] });
render(() => <Block id="drag-block" />, document.getElementById("root")!);
