import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { closeDatePicker, datePicker } from "../ui";
import { Block } from "./Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  closeDatePicker();
  resetStore();
  document.body.innerHTML = "";
});

describe("scheduled date chrome", () => {
  it("stays clickable when the block has body text after the planning line (#75)", () => {
    const id = "scheduled-with-body";
    const page: FeedPage = {
      name: "Schedule",
      kind: "page",
      title: "Schedule",
      preBlock: null,
      roots: [id],
      format: "md",
      readOnly: false,
      guide: false,
    };
    const node: StoreNode = {
      id,
      raw: "Task\nSCHEDULED: <2026-07-13 Mon>\nnotes after the schedule",
      collapsed: false,
      parent: null,
      page: page.name,
      children: [],
    };
    setDoc({ byId: { [id]: node }, pages: [page], feed: [], loaded: true });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id={id} />, host);
    try {
      const chip = host.querySelector<HTMLElement>(".date-chip.scheduled");
      expect(chip?.textContent).toContain("2026-07-13 Mon");
      chip?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      chip?.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      chip?.click();
      expect(datePicker()).toMatchObject({ blockId: id, which: "scheduled" });
    } finally {
      dispose();
    }
  });
});
