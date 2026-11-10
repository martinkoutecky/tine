import { createEffect, createSignal, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { cm6EditorEnabled, cm6EditorLoading } from "../editor/cm6Setting";
import type { Cm6BlockEditor } from "./Cm6BlockEditor";
import { focusMountedEditor } from "../editor/surfaceFocus";
import { noteSurfaceFocused } from "../editorController";
import type { propertyEditorSession } from "../editor/propertySession";
import type { OutlineScope } from "../document";
import { reportUiFailure } from "../uiFailure";

let loadedEditor: typeof Cm6BlockEditor | undefined;
let loading: Promise<void> | undefined;

/** Load the optional engine once per window. No graph reads, writes or IPC;
 * one asset import on first use, O(1) thereafter. Reports asset failures; the
 * promise settles after the attempt, leaving the classic engine available. */
export function preloadCm6Editor(): Promise<void> {
  return loading ??= import("./Cm6BlockEditor").then(
    module => { loadedEditor = module.Cm6BlockEditor; }, error => reportUiFailure("editor-load", error));
}

/** Ordered browser input received before the editor mounts. The engine supplies
 * its ordinary insertion path and keyboard target after applying the caret. */
type InputKind = "type" | "paste" | "composition";
export type PendingEditorInput = (insert: (text: string, target: HTMLElement, kind: InputKind) => void, target: HTMLElement) => void;

/** Choose once per activation. Await the stored choice (at most five seconds),
 * then the optional engine. A cold activation buffers native input, never mounts
 * another engine, and leaves the caret request for the chosen engine to take. */
export function EditorGate(props: {
  id: string; surface: string; scope: OutlineScope | null;
  propertySession?: ReturnType<typeof propertyEditorSession>;
  classic: (pending?: PendingEditorInput) => JSX.Element;
}): JSX.Element {
  const [choiceReady, setChoiceReady] = createSignal(!untrack(cm6EditorLoading));
  const [engine, setEngine] = createSignal(loadedEditor);
  const [failed, setFailed] = createSignal(false);
  const [composing, setComposing] = createSignal(false);
  const input: Array<{ text: string; kind: InputKind } | KeyboardEvent | ClipboardEvent> = [];
  let alive = true;
  const timer = choiceReady() ? undefined : setTimeout(() => setChoiceReady(true), 5000);
  onCleanup(() => { alive = false; clearTimeout(timer); });
  createEffect(() => { if (!cm6EditorLoading()) setChoiceReady(true); });
  let chosen: boolean | undefined;
  const ready = () => {
    if (!choiceReady()) return false;
    chosen ??= untrack(cm6EditorEnabled);
    return (!chosen || engine() || failed()) && !composing();
  };
  createEffect(() => {
    if (!choiceReady() || !untrack(cm6EditorEnabled) || untrack(engine)) return;
    void preloadCm6Editor().then(() => {
      if (!alive) return;
      const loaded = loadedEditor;
      if (loaded) setEngine(() => loaded);
      else setFailed(true);
    });
  });
  const replay: PendingEditorInput = (insert, target) => {
    const events = input.splice(0);
    let next = 0;
    const deliver = () => {
      if (next === events.length) return;
      const event = events[next++];
      // A buffered Enter can mount the next block before the following input.
      const focused = target.ownerDocument.activeElement as HTMLElement | null;
      if (!focused?.matches('textarea.block-editor, [data-cm6-editor]')) return;
      if ("text" in event) insert(event.text, focused, event.kind);
      else if (event.type === "paste") {
        focused.dispatchEvent(event);
        const text = (event as ClipboardEvent).clipboardData?.getData("text/plain");
        if (!event.defaultPrevented && text) insert(text, focused, "paste");
      }
      else focused.dispatchEvent(new KeyboardEvent("keydown", event));
      // Structural keys can mount a new editor after this mount hook returns.
      queueMicrotask(deliver);
    };
    deliver();
  };
  function WaitingInput(): JSX.Element {
    let ref!: HTMLTextAreaElement;
    const collect = () => {
      if (ref.value) { input.push({ text: ref.value, kind: composing() ? "composition" : "type" }); ref.value = ""; }
    };
    const finishComposition = () => {
      if (!ref.isConnected) return;
      collect(); setComposing(false);
    };
    onMount(() => focusMountedEditor(props.id, props.surface, () => ref, () => ref.focus({ preventScroll: true })));
    return <textarea ref={ref} data-block-editor="pending" aria-busy="true" aria-label="" style={{ position: "absolute", opacity: 0, width: "1px", height: "1px", "pointer-events": "none" }}
      onFocus={() => noteSurfaceFocused(props.surface)}
      onInput={() => { if (ref.isConnected && !composing()) collect(); }}
      onCompositionStart={() => { if (ref.isConnected) setComposing(true); }}
      onCompositionEnd={finishComposition}
      onBlur={finishComposition}
      onPaste={event => {
        event.preventDefault();
        const data = event.clipboardData;
        const types = Array.from(data?.types ?? []);
        const values = new Map(types.map(type => [type, data!.getData(type)]));
        const files = Array.from(data?.files ?? []);
        const items = Array.from(data?.items ?? [], item => {
          const file = item.getAsFile();
          return { kind: item.kind, type: item.type, getAsFile: () => file };
        });
        const paste = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
        Object.defineProperty(paste, "clipboardData", { value: { types, files, items, getData: (type: string) => values.get(type) ?? "" } });
        input.push(paste);
      }}
      onKeyDown={event => {
        event.stopPropagation();
        if (event.isComposing || event.key === "Process" || event.key === "Dead") return;
        if (event.key.length === 1 && !event.metaKey && (!event.ctrlKey || event.altKey)) return;
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v") return;
        event.preventDefault();
        input.push(event);
      }} />;
  }
  function ChosenEngine(): JSX.Element {
    const Engine = engine();
    return chosen && Engine && !failed() ? <Engine id={props.id} surface={props.surface} scope={props.scope} propertySession={props.propertySession} pendingInput={replay} />
      : props.classic(replay);
  }
  return <Show when={ready()} fallback={<WaitingInput />}><ChosenEngine /></Show>;
}
