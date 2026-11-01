import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { bumpGraphEpoch, setGraphMeta } from "../graphSession";
import { backend } from "../backend";
import { type GraphMeta } from "../types";

afterEach(() => { closeSettings(); setGraphMeta(null); vi.restoreAllMocks(); document.body.innerHTML = ""; });

// GH #808: the note naming the Journal sections page sat in the Agenda window setting's control column,
// which never shrinks, so the sentence ran off the Settings pane and "Open page" was clipped. jsdom has no
// layout (scripts/check-gh808-settings-note.mjs measures the real thing); this pins the structure that
// fixes it: the note belongs to the setting's full-width, wrapping hint, never to the control column.
it("GH #808: the sections-page note and its Open page link sit in the wrapping hint, not the control column", async () => {
  bumpGraphEpoch();
  vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "page", pages: [{ name: "Journal sections" }], diagnostics: [], report: { ran: [], ignored: [], supported: true }, total: 1, exceeded: false } as never);
  setGraphMeta({ root: "/fixture", journal_config_diagnostics: [] } as unknown as GraphMeta);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("journals");
    const link = await vi.waitFor(() => {
      const found = [...root.querySelectorAll<HTMLButtonElement>(".settings-link")].find((b) => b.textContent === "Open page");
      expect(found).toBeTruthy();
      return found!;
    });
    expect(link.closest(".settings-field-control")).toBeNull();
    const hint = link.closest(".settings-field-hint");
    expect(hint).not.toBeNull();
    expect(hint!.closest(".settings-field")?.getAttribute("data-setting-label")).toBe("Agenda window");
    expect(hint!.textContent).toContain("Journal sections");
  } finally { dispose(); }
});
