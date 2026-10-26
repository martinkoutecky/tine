import { bindingCurrent, captureBinding, graphScopedSignal } from "./binding";

// Open sheet property inputs are DOM drafts, unlike ordinary block editors
// whose keystrokes already live in the document. Keep their owning result
// surface mounted until commit/cancel; provenance still guards every action.
const [drafts, setDrafts] = graphScopedSignal<readonly { id: string }[]>();
export const resultDrafts = () => drafts() ?? [];
export function retainResultDraft(id: string): () => void {
  const binding = captureBinding();
  const draft = { id };
  setDrafts([...resultDrafts(), draft]);
  return () => { if (bindingCurrent(binding)) setDrafts(resultDrafts().filter(item => item !== draft)); };
}
