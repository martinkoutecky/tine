import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { HomePageSetting } from "./HomePageSetting";
import { backend } from "../backend";
import { bumpGraphEpoch, graphMeta, setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

afterEach(() => { vi.restoreAllMocks(); setGraphMeta(null); document.body.innerHTML = ""; });

it("chooses an existing page through Settings and writes default-home", async () => {
  setGraphMeta({ root: "/graph", default_home: null } as GraphMeta);
  vi.spyOn(backend(), "quickSwitch").mockResolvedValue([
    { name: "Home", kind: "page", date_key: null, path: "pages/Home.md" },
    { name: "Today", kind: "journal", date_key: 20260929, path: "journals/2026_09_29.md" },
  ]);
  const write = vi.spyOn(backend(), "setDefaultHome").mockResolvedValue();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <HomePageSetting />, root);
  await vi.waitFor(() => expect(root.querySelector(".settings-btn")).not.toBeNull());
  expect(root.textContent).not.toContain("Today");
  root.querySelector<HTMLButtonElement>(".settings-btn")!.click();
  await vi.waitFor(() => expect(graphMeta()?.default_home).toBe("Home"));
  expect(write).toHaveBeenCalledWith("Home");
  dispose();
});

it("asks again when the graph epoch outruns an outstanding lookup", async () => {
  setGraphMeta({ root: "/graph", default_home: null } as GraphMeta);
  const home = [{ name: "Home", kind: "page" as const, date_key: null, path: "pages/Home.md" }];
  let reply!: (pages: typeof home) => void;
  const lookup = vi.spyOn(backend(), "quickSwitch")
    .mockImplementationOnce(() => new Promise((resolve) => { reply = resolve; }))
    .mockResolvedValue(home);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <HomePageSetting />, root);
  try {
    await vi.waitFor(() => expect(lookup).toHaveBeenCalledTimes(1));
    bumpGraphEpoch();
    reply([]);
    await vi.waitFor(() => expect(root.querySelector(".settings-btn")?.textContent).toBe("Home"));
    expect(lookup).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});
