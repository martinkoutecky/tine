import { EditorSelection } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { vi } from "vitest";

/** Supply the CM equivalent of a fixture's fixed-width textarea mirror rows.
 * jsdom has no Range layout. This tests navigation given those rows, while
 * real-browser journeys establish that CM resolves actual wrapped rows. */
export function mockCm6VisualRows(width: number): void {
  vi.spyOn(EditorView.prototype, "moveToLineBoundary").mockImplementation(function (this: EditorView, range, forward) {
    const line = this.state.doc.lineAt(range.head);
    const occupied = range.head === this.state.doc.length && range.head > 0 ? range.head - 1 : range.head;
    const row = Math.floor(occupied / width) * width;
    return EditorSelection.cursor(forward ? Math.min(line.to, row + width) : Math.max(line.from, row));
  });
}
