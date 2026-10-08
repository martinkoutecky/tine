import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";
import { openSettings, closeSettings } from "../ui";
import { toasts, setToasts } from "../toasts";
import { Settings } from "./Settings";

afterEach(() => { closeSettings(); setGraphMeta(null); setToasts([]); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("shows unavailable templates with Retry rather than asserting the configured template is missing", async () => {
  setToasts([]);
  setGraphMeta({ root: "/tmp/template-read-failure", default_journal_template: "Daily" } as GraphMeta);
  const read = vi.spyOn(backend(), "listTemplates").mockRejectedValue(new Error("io:PermissionDenied"));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Settings />, host);
  openSettings("journals");
  await vi.waitFor(() => expect(read).toHaveBeenCalled());
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(host.textContent).not.toContain("Daily (not found)");
  expect(host.querySelector(".settings-jtmpl .resource-failure")?.textContent).toContain("Retry");
  expect(toasts().some((t) => t.kind === "error")).toBe(true);
  read.mockResolvedValue([]);
  host.querySelector<HTMLButtonElement>(".settings-jtmpl .resource-failure-retry")!.click();
  await vi.waitFor(() => expect(host.textContent).toContain("Daily (not found)"));
  dispose();
});
