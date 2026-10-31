import { StateEffect, type EditorState, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";

export interface CodeHighlightRegion { from: number; to: number; lang: string }
export interface CodeHighlightRequest {
  revision: number;
  regions: (CodeHighlightRegion & { code: string })[];
}
export interface CodeHighlightReply {
  revision: number;
  regions: (CodeHighlightRegion & { html: string })[];
}

export const refreshCodeHighlight = StateEffect.define<null>();
const highlighted = StateEffect.define<CodeHighlightReply>();

/** The worker returns highlight.js's escaped HTML, never editable DOM. Walk its
 * text to recover UTF-16 mark ranges, retaining nested scopes and the view's
 * existing hljs classes. The document itself is never replaced. */
function tokenDecorations(document: Document, reply: CodeHighlightReply): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  for (const region of reply.regions) {
    const template = document.createElement("template");
    template.innerHTML = region.html;
    let offset = region.from;
    const visit = (node: Node) => {
      const from = offset;
      if (node.nodeType === 3) offset += node.textContent?.length ?? 0;
      else node.childNodes.forEach(visit);
      if (node.nodeType === 1 && from < offset && offset <= region.to) {
        const classes = (node as Element).getAttribute("class");
        if (classes) ranges.push(Decoration.mark({ class: classes }).range(from, offset));
      }
    };
    visit(template.content);
  }
  return Decoration.set(ranges, true);
}

/** Keep grammar work off the input thread, including highlightAuto for unknown
 * languages (the same policy and common bundle as the rendered view). Coalesce
 * bursts, map old tokens immediately, and reject replies for superseded text
 * or coordinate projections. One worker exists only while this editor needs it. */
export function cm6CodeHighlight(regions: (state: EditorState) => CodeHighlightRegion[]) {
  return ViewPlugin.fromClass(class {
    decorations = Decoration.none;
    worker: Worker | undefined;
    timer: ReturnType<typeof setTimeout> | undefined;
    revision = 0;
    destroyed = false;
    busy = false;
    pending: CodeHighlightRequest | undefined;
    unavailable = false;

    constructor(readonly view: EditorView) { this.schedule(); }

    update(update: ViewUpdate) {
      this.decorations = this.decorations.map(update.changes);
      if (update.docChanged || update.transactions.some(tr => tr.effects.some(effect => effect.is(refreshCodeHighlight)))) {
        this.schedule();
      }
      for (const tr of update.transactions) for (const effect of tr.effects) {
        if (effect.is(highlighted) && effect.value.revision === this.revision) {
          this.decorations = tokenDecorations(this.view.dom.ownerDocument, effect.value);
        }
      }
    }

    schedule() {
      const revision = ++this.revision;
      clearTimeout(this.timer);
      this.pending = undefined;
      const spans = regions(this.view.state);
      if (!spans.length) { this.decorations = Decoration.none; return; }
      this.timer = setTimeout(() => {
        if (this.destroyed || this.unavailable || typeof Worker === "undefined") return;
        this.pending = { revision, regions: spans.map(span => ({ ...span,
          code: this.view.state.sliceDoc(span.from, span.to),
        })) };
        this.send();
      }, 75);
    }

    send() {
      if (this.busy || !this.pending) return;
      try {
        if (!this.worker) {
          this.worker = new Worker(new URL("./cm6CodeHighlight.worker.ts", import.meta.url), { type: "module" });
          this.worker.onmessage = (event: MessageEvent<CodeHighlightReply>) => {
            this.busy = false;
            if (this.destroyed) return;
            if (event.data.revision === this.revision) this.view.dispatch({ effects: highlighted.of(event.data) });
            this.send();
          };
          this.worker.onerror = () => this.disable();
        }
        this.busy = true;
        this.worker.postMessage(this.pending);
        this.pending = undefined;
      } catch { this.disable(); }
    }

    // A failed optional highlighter must never take down the editing surface.
    disable() {
      this.unavailable = true;
      this.pending = undefined;
      this.worker?.terminate();
    }

    destroy() {
      this.destroyed = true;
      clearTimeout(this.timer);
      this.worker?.terminate();
    }
  }, { decorations: plugin => plugin.decorations });
}
