import { killLineBefore, killLineAfter, killWordBackward, killWordForward, wordBackward, wordForward, type Edit } from "./format";

// Command semantics are shared; each editor applies edits and caret moves to
// its own buffer. A motion must not publish a text edit or create an undo unit.
export const textCommands: Readonly<Record<string, (text: string, start: number) => Edit | number>> = {
  "editor/clear-block": () => ({ text: "", start: 0, end: 0 }),
  "editor/kill-line-before": killLineBefore,
  "editor/kill-line-after": killLineAfter,
  "editor/backward-kill-word": killWordBackward,
  "editor/forward-kill-word": killWordForward,
  "editor/backward-word": wordBackward,
  "editor/forward-word": wordForward,
};

export function textCommandHandlers(ctx: {
  text(): string; start(): number; apply(edit: Edit): void; move(pos: number): void;
}): Record<string, () => boolean> {
  return Object.fromEntries(Object.entries(textCommands).map(([id, operation]) => [id, () => {
    const result = operation(ctx.text(), ctx.start());
    if (typeof result === "number") ctx.move(result);
    else ctx.apply(result);
    return true;
  }]));
}
