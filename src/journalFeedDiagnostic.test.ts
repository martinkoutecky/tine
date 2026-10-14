import { expect, it, vi } from "vitest";
import { backend } from "./backend";
import { journalFeedCause, recordFeedReason } from "./journalFeedDiagnostic";

it.each([
  [null, "empty-rejection"],
  [undefined, "empty-rejection"],
  [new Error("private graph content"), "other-rejection"],
  [{ message: "private graph content" }, "other-rejection"],
])("classifies a journal rejection into a closed token: %s", (error, expected) => {
  expect(journalFeedCause(error)).toBe(expected);
});

it("records the reason and rejection category without graph text, and tolerates recorder failure", async () => {
  const sink = vi.spyOn(backend(), "diagnosticFrontendEvent").mockRejectedValue(new Error("recorder unavailable"));
  try {
    recordFeedReason("template-error", "secret/path: Permission denied (os error 13)");
    expect(sink).toHaveBeenCalledWith("journal_feed", { journalFeedReason: "template-error", journalFeedCause: "other-rejection" });
    expect(JSON.stringify(sink.mock.calls)).not.toContain("secret");
    await Promise.resolve();
  } finally { sink.mockRestore(); }
});
