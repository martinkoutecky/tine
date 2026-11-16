import type { Backend } from "./backendInterface";

/** Process-memory capture custody for one browser backend instance. */
export function createMockCaptureReceipts(): Backend["captureReceipt"] {
  const receipts = new Map<string, "pending" | "saved">();
  return async (id, action) => {
    if (action === "status") return receipts.get(id) ?? "missing";
    if (action === "release" || action === "forget") {
      if (receipts.get(id) !== "saved") receipts.delete(id);
      return action === "forget" ? receipts.get(id) ?? "missing" : "saved";
    }
    if (action === "saved") { receipts.set(id, "saved"); return "saved"; }
    const prior = receipts.get(id);
    if (prior) return prior;
    receipts.set(id, "pending");
    return "new";
  };
}
