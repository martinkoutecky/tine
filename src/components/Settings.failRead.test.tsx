import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";
import { openSettings, closeSettings } from "../ui";
import { toasts, setToasts } from "../toasts";
import { Settings } from "./Settings";
import * as router from "../router";

afterEach(() => { closeSettings(); setGraphMeta(null); setToasts([]); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("opens the journal picker without requesting template bodies", async () => {
  setGraphMeta({ root: "/tmp/template-names", default_journal_template: "meeting" } as GraphMeta);
  const bodies = vi.spyOn(backend(), "listTemplates");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Settings />, host);
  try {
    openSettings("journals");
    await vi.waitFor(() => expect(host.querySelector(".settings-jtmpl select")?.textContent).toContain("meeting"));
    expect(bodies).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("keeps names in order and resolves only the chosen definition when Edit is clicked", async () => {
  setGraphMeta({ root: "/tmp/template-names-edit", default_journal_template: "Daily" } as GraphMeta);
  vi.spyOn(backend(), "listTemplateNames").mockResolvedValue(["Daily", "Weekly", "Daily"]);
  const bodies = vi.spyOn(backend(), "listTemplates").mockResolvedValue([{ name: "Weekly", page: "Plans", kind: "page", blocks: [] }]);
  const write = vi.spyOn(backend(), "setDefaultJournalTemplate").mockResolvedValue(undefined);
  const open = vi.spyOn(router, "openPage").mockImplementation(() => {});
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Settings />, host);
  try {
    openSettings("journals");
    await vi.waitFor(() => expect(host.querySelectorAll(".settings-jtmpl option")).toHaveLength(4));
    const select = host.querySelector<HTMLSelectElement>(".settings-jtmpl select")!;
    expect([...select.options].map(o => o.value)).toEqual(["", "Daily", "Weekly", "Daily"]);
    select.value = "Weekly"; select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith("Weekly"));
    expect(bodies).not.toHaveBeenCalled();
    host.querySelector<HTMLButtonElement>(".settings-jtmpl .settings-link")!.click();
    await vi.waitFor(() => expect(open).toHaveBeenCalledWith("Plans", "page"));
    expect(bodies.mock.calls).toEqual([["Weekly"]]);
  } finally { dispose(); }
});

it("refreshes the names when Edit finds the chosen definition gone", async () => {
  setGraphMeta({ root: "/tmp/template-names-gone", default_journal_template: "Daily" } as GraphMeta);
  const names = vi.spyOn(backend(), "listTemplateNames").mockResolvedValueOnce(["Daily"]).mockResolvedValue([]);
  vi.spyOn(backend(), "listTemplates").mockResolvedValue([]);
  const open = vi.spyOn(router, "openPage").mockImplementation(() => {});
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Settings />, host);
  try {
    openSettings("journals");
    await vi.waitFor(() => expect(host.querySelector(".settings-jtmpl .settings-link")).not.toBeNull());
    host.querySelector<HTMLButtonElement>(".settings-jtmpl .settings-link")!.click();
    await vi.waitFor(() => expect(names).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(host.querySelector(".settings-jtmpl .settings-link")).toBeNull());
    expect(open).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("reports Edit read failures and ignores an Edit result after the selection changes", async () => {
  setGraphMeta({ root: "/tmp/template-names-late", default_journal_template: "Daily" } as GraphMeta);
  vi.spyOn(backend(), "listTemplateNames").mockResolvedValue(["Daily", "Weekly"]);
  const bodies = vi.spyOn(backend(), "listTemplates").mockRejectedValueOnce(new Error("io:PermissionDenied"));
  const open = vi.spyOn(router, "openPage").mockImplementation(() => {});
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Settings />, host);
  try {
    openSettings("journals");
    await vi.waitFor(() => expect(host.querySelector(".settings-jtmpl .settings-link")).not.toBeNull());
    host.querySelector<HTMLButtonElement>(".settings-jtmpl .settings-link")!.click();
    await vi.waitFor(() => expect(toasts().some(t => t.kind === "error")).toBe(true));
    expect(open).not.toHaveBeenCalled();
    // Resolve after a new selection, still in the same mounted settings pane.
    let resolve!: (value: import("../types").TemplateDto[]) => void;
    bodies.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
    host.querySelector<HTMLButtonElement>(".settings-jtmpl .settings-link")!.click();
    setGraphMeta({ ...graphMeta()!, default_journal_template: "Weekly" });
    resolve([{ name: "Daily", page: "Old selection", kind: "page", blocks: [] }]);
    await new Promise(r => setTimeout(r, 0));
    expect(open).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("shows unavailable templates with Retry rather than asserting the configured template is missing", async () => {
  setToasts([]);
  setGraphMeta({ root: "/tmp/template-read-failure", default_journal_template: "Daily" } as GraphMeta);
  const read = vi.spyOn(backend(), "listTemplateNames").mockRejectedValue(new Error("io:PermissionDenied"));
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
