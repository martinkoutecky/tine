import { bindingCurrent, captureBinding, graphScopedSignal } from "./binding";

// Property inputs and promoted preambles can own an editor whose identity is
// absent from the original answer. Keep that result surface mounted until the
// draft finishes; provenance still guards every source action.
const [drafts, setDrafts] = graphScopedSignal<readonly { id: string }[]>();
export const resultDrafts = () => drafts() ?? [];
export function retainResultDraft(id: string): () => void {
  const binding = captureBinding();
  const draft = { id };
  setDrafts([...resultDrafts(), draft]);
  return () => { if (bindingCurrent(binding)) setDrafts(resultDrafts().filter(item => item !== draft)); };
}
