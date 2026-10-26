import { textCommandHandlers } from "./textCommands";
import type { EditorView } from "@codemirror/view";

type CommandHandlers<Args extends unknown[] = []> = Readonly<Record<string, (...args: Args) => boolean>>;

export function guardedCommands<Args extends unknown[]>(available: (...args: Args) => boolean, handlers: CommandHandlers<Args>): CommandHandlers<Args> {
  return Object.fromEntries(Object.entries(handlers).map(([id, run]) => [id, (...args: Args) => available(...args) && run(...args)]));
}

/** The command registry used by both keyboard and focused-editor dispatch.
 * Keep configured command IDs here, never physical key exceptions. The registry
 * completeness test requires every editor-scoped keybinding to have a route. */
export function cm6CommandHandlers(ctx: {
  text: Parameters<typeof textCommandHandlers>[0];
  view(): EditorView;
  structure: CommandHandlers<[EditorView]>;
  formatting: CommandHandlers;
  completion: CommandHandlers;
  clipboard: CommandHandlers<[EditorView]>;
  capture(): boolean;
  upload(): void;
  photo(): void;
  voice(): void;
}): Record<string, () => boolean> {
  const bindView = (handlers: CommandHandlers<[EditorView]>) =>
    Object.fromEntries(Object.entries(handlers).map(([id, run]) => [id, () => run(ctx.view())]));
  return {
    ...textCommandHandlers(ctx.text),
    ...bindView(ctx.structure), ...ctx.formatting, ...ctx.completion, ...bindView(ctx.clipboard),
    "editor/quick-capture-file": ctx.capture,
    "editor/upload-asset": () => { ctx.upload(); return true; },
    "editor/capture-photo": () => { ctx.photo(); return true; },
    "editor/voice-memo": () => { ctx.voice(); return true; },
  };
}
