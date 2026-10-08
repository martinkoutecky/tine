import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";

export type GraphNameRequest = {
  suggestion: string;
  create: (name: string) => Promise<string | null>;
  finish: (root: string | null) => void;
};
let pendingRequest: GraphNameRequest | null = null;
// The scoped getter is already stale when invalidation callbacks run.
clearOnBindingInvalidated(() => pendingRequest?.finish(null));
export const [graphNameRequest, setGraphNameRequest] = graphScopedSignal<GraphNameRequest>();

/** One pending creation prompt; its caller owns native reads and writes. */
export function askGraphName(suggestion: string, create: GraphNameRequest["create"]): Promise<string | null> {
  if (pendingRequest) return Promise.resolve(null);
  return new Promise(resolve => {
    const request: GraphNameRequest = { suggestion, create, finish(root) {
      if (pendingRequest !== request) return;
      pendingRequest = null;
      setGraphNameRequest(null);
      resolve(root);
    } };
    pendingRequest = request;
    setGraphNameRequest(request);
  });
}
