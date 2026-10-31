import hljs from "highlight.js/lib/common";
import type { CodeHighlightReply, CodeHighlightRequest } from "./cm6CodeHighlight";

self.onmessage = (event: MessageEvent<CodeHighlightRequest>) => {
  const { revision, regions } = event.data;
  self.postMessage({ revision, regions: regions.map(({ code, ...region }) => {
    let html = "";
    try {
      html = region.lang && hljs.getLanguage(region.lang)
        ? hljs.highlight(code, { language: region.lang }).value : hljs.highlightAuto(code).value;
    } catch { /* Leave the editable source plain if a grammar fails. */ }
    return { ...region, html };
  }) } satisfies CodeHighlightReply);
};
