import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { readPageProperty, resetStore } from "../document";
import { setDoc } from "../document/model";
import { closePageProps, openPageProps } from "../ui";
import { PageProps } from "./PageProps";

function load() {
  setDoc({
    loaded: true, feed: ["P"], byId: {},
    pages: [{ name: "P", kind: "page", title: "P", preBlock: null, roots: [], format: "md", readOnly: false, guide: false }],
  });
}

afterEach(() => {
  closePageProps();
  resetStore();
  document.body.innerHTML = "";
});

it("does not write a page property from an old graph panel into a colliding page", () => {
  load();
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <PageProps />, host);
  openPageProps("P", 100, 100);
  const alias = host.querySelector<HTMLInputElement>('.pp-input[placeholder="comma, separated"]')!;
  alias.value = "Old alias";
  alias.dispatchEvent(new Event("input", { bubbles: true }));
  resetStore();
  load();
  alias.dispatchEvent(new Event("blur"));
  expect(readPageProperty("P", "alias")).toBeNull();
  dispose();
});
