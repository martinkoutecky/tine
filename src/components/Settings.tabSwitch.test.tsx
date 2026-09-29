import { afterEach, describe, expect, it } from "vitest";
import { Suspense } from "solid-js";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
});

// GH #409. Settings is mounted lazily under a fallback-less <Suspense> in
// App.tsx, so ANY resource read inside the dialog suspends the whole dialog and
// the user sees Settings vanish and come back. Reproduce that exact wrapper —
// without it the panels merely render empty and nothing looks wrong.
function mount() {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(
    () => (
      <Suspense fallback={<div class="app-suspense-fallback" />}>
        <Settings />
      </Suspense>
    ),
    root,
  );
  return { root, dispose };
}

const navItem = (root: HTMLElement, label: string) =>
  [...root.querySelectorAll<HTMLButtonElement>(".settings-nav-item")].find(
    (button) => button.textContent === label,
  )!;

describe("switching Settings sections (GH #409)", () => {
  // The reporter's three: "the entire Settings interface disappears and then
  // reopens … as if the Settings view is being fully closed and recreated".
  for (const label of ["Journals", "Backups & recovery", "Graph"]) {
    it(`keeps the dialog on screen while opening ${label}`, async () => {
      const { root, dispose } = mount();
      openSettings();
      await tick();
      expect(root.querySelector(".settings-modal")).toBeTruthy();

      navItem(root, label).click();
      // Checked synchronously: the flash is one frame wide and a settled
      // assertion cannot see it.
      expect(root.querySelector(".settings-modal")).toBeTruthy();
      expect(root.querySelector(".app-suspense-fallback")).toBeNull();

      await tick();
      expect(root.querySelector(".settings-modal")).toBeTruthy();
      dispose();
    });
  }

  it("keeps the section list and the search box across a switch that loads", async () => {
    const { root, dispose } = mount();
    openSettings();
    await tick();
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "journal";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await tick();

    navItem(root, "Journals").click();
    // The chrome is outside the boundary, so it survives the load rather than
    // being torn down and rebuilt with a cleared search box.
    expect(root.querySelector(".settings-nav-item.active")!.textContent).toBe("Journals");
    expect(root.querySelector<HTMLInputElement>(".settings-search-input")!.value).toBe("journal");

    await tick();
    expect(root.querySelector(".settings-nav-item.active")!.textContent).toBe("Journals");
    dispose();
  });
});
