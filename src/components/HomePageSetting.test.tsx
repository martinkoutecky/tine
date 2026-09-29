import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { HomePageSetting } from "./HomePageSetting";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";
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
