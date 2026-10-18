import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { DialogActions } from "./DialogActions";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.innerHTML = ""; });

function mount(view: () => any) {
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(view, host);
  return host;
}

it("renders aux, then Cancel, then the primary action, whatever order the props were written in", () => {
  const host = mount(() => (
    <DialogActions
      primary={<button>Save</button>}
      cancel={<button>Cancel</button>}
      aux={<button>Reload</button>} />
  ));
  expect([...host.querySelectorAll(".dialog-actions button")].map((b) => b.textContent)).toEqual(["Reload", "Cancel", "Save"]);
  expect(host.querySelector(".dialog-actions-aux")!.textContent).toBe("Reload");
});

it("needs only a primary: no aux wrapper and no Cancel when they are absent", () => {
  const host = mount(() => <DialogActions primary={<button>Done</button>} />);
  expect(host.querySelector(".dialog-actions-aux")).toBeNull();
  expect([...host.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Done"]);
});

it("keeps a per-dialog class for sizing without taking over the order", () => {
  const host = mount(() => <DialogActions class="share-foot" cancel={<button>Cancel</button>} primary={<button>Export</button>} />);
  const row = host.querySelector(".dialog-actions")!;
  expect(row.classList.contains("share-foot")).toBe(true);
  const main = row.querySelector(".dialog-actions-main")!;
  expect(row.lastElementChild).toBe(main);
  expect(main.lastElementChild!.textContent).toBe("Export");
  expect(main.firstElementChild!.textContent).toBe("Cancel");
});

it("keeps Cancel and the primary action in one group, so they can never wrap apart", () => {
  const host = mount(() => (
    <DialogActions aux={<button>Reload</button>} cancel={<button>Cancel</button>} primary={<button>Save</button>} />
  ));
  const main = host.querySelector(".dialog-actions-main")!;
  expect([...main.children].map((b) => b.textContent)).toEqual(["Cancel", "Save"]);
  expect(main.previousElementSibling!.className).toBe("dialog-actions-aux");
});
