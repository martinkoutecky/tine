import { createSignal, type Accessor } from "solid-js";

export type MobileEditorCommandId =
  | "editor/outdent"
  | "editor/indent"
  | "editor/move-block-up"
  | "editor/move-block-down"
  | "editor/soft-newline"
  | "editor/cycle-todo"
  | "editor/upload-asset"
  | "editor/capture-photo"
  | "editor/voice-memo"
  | "editor/open-date-picker"
  | "editor/insert-page-ref"
  | "editor/insert-block-ref"
  | "editor/open-slash-menu";

export interface FocusedEditorCommandBridge {
  blockId: string;
  dispatch(command: MobileEditorCommandId): boolean;
  blur(): void;
}

const [focusedEditorBridge, setFocusedEditorBridge] =
  createSignal<FocusedEditorCommandBridge | null>(null);

export const focusedEditorCommandBridge: Accessor<FocusedEditorCommandBridge | null> =
  focusedEditorBridge;

export function registerFocusedEditorCommandBridge(
  bridge: FocusedEditorCommandBridge
): () => void {
  setFocusedEditorBridge(bridge);
  return () => {
    setFocusedEditorBridge((cur) => cur === bridge ? null : cur);
  };
}

export function dispatchFocusedEditorCommand(command: MobileEditorCommandId): boolean {
  return focusedEditorBridge()?.dispatch(command) ?? false;
}

export function blurFocusedEditor(): boolean {
  const bridge = focusedEditorBridge();
  if (!bridge) return false;
  bridge.blur();
  return true;
}
