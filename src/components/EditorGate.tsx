import { createEffect, createSignal, onCleanup, Show, untrack, type JSX } from "solid-js";
import { cm6EditorEnabled, cm6EditorLoading } from "../editor/cm6Setting";
import type { Cm6BlockEditor } from "./Cm6BlockEditor";
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
    module => { loadedEditor = module.Cm6BlockEditor; }, error => { reportUiFailure("editor-load", error); });
}

/** Await the stored choice (at most five seconds), then choose once per
 * activation. An enabled engine still loading uses the native classic editor
 * for this activation; the completed import serves the next activation. */
export function EditorGate(props: {
  id: string; surface: string; scope: OutlineScope | null;
  propertySession?: ReturnType<typeof propertyEditorSession>;
  classic: () => JSX.Element;
}): JSX.Element {
  const [choiceReady, setChoiceReady] = createSignal(!untrack(cm6EditorLoading));
  const timer = choiceReady() ? undefined : setTimeout(() => setChoiceReady(true), 5000);
  onCleanup(() => clearTimeout(timer));
  createEffect(() => { if (!cm6EditorLoading()) setChoiceReady(true); });
  function ChosenEngine(): JSX.Element {
    const enabled = untrack(cm6EditorEnabled);
    const Engine = enabled ? loadedEditor : undefined;
    if (enabled) void preloadCm6Editor();
    return Engine ? <Engine id={props.id} surface={props.surface} scope={props.scope} propertySession={props.propertySession} />
      : props.classic();
  }
  return <Show when={choiceReady()}><ChosenEngine /></Show>;
}
