import { afterEach, expect, it } from "vitest";
import { createComputed, createRoot, createSignal } from "solid-js";
import { resetStore } from "../document";
import { setDoc, type Node } from "../document/model";
import { createOutlineWindowGeometry, createOutlineWindowModel } from "./outlineWindowModel";

const row = (id: string, children: string[] = []): Node => ({
  id, raw: id, children, collapsed: false, parent: null, page: "Window",
});
afterEach(resetStore);

it("keeps reveal membership stable on text edits while updating multiline spacer geometry", () => {
  setDoc("byId", { root: row("root", ["child"]), child: row("child") });
  createRoot(dispose => {
    const model = createOutlineWindowModel(() => ["root"], () => (_id, _page, stored) => stored);
    const geometry = createOutlineWindowGeometry(model, () => 900, () => "16px sans-serif");
    let registrations = 0;
    createComputed(() => { model(); registrations++; });
    const before = model(), version = geometry().version();
    expect(geometry().estimate()).toBe(56);
    setDoc("byId", "child", "raw", "one\ntwo\nthree");
    expect(model()).toBe(before);
    expect(registrations).toBe(1);
    expect(geometry().estimate()).toBe(108);
    expect(geometry().version()).toBeGreaterThan(version);
    setDoc("byId", "root", "collapsed", true);
    expect(model().all).toEqual(["root", "child"]);
    expect(geometry().estimate()).toBe(28);
    setDoc("byId", "child", "raw", "one\ntwo\nthree\nfour");
    expect(geometry().estimate()).toBe(28);
    setDoc("byId", "root", "collapsed", false);
    expect(geometry().estimate()).toBe(134);
    dispose();
  });
});

it("rebuilds geometry and reveal membership for structure and occurrence folds, disposing old observers", () => {
  setDoc("byId", { root: row("root", ["child"]), child: row("child"), added: row("added") });
  createRoot(dispose => {
    const [folded, setFolded] = createSignal(false);
    const model = createOutlineWindowModel(() => ["root"], () => () => folded());
    const geometry = createOutlineWindowGeometry(model, () => 900, () => "16px sans-serif");
    const old = geometry();
    setDoc("byId", "root", "children", ["added"]);
    expect(model().members.has("child")).toBe(false);
    expect(model().members.has("added")).toBe(true);
    setDoc("byId", "child", "raw", "removed\nrow");
    expect(old.estimate()).toBe(56);
    expect(geometry().estimate()).toBe(56);
    setDoc("byId", "added", "raw", "new\nrow");
    expect(geometry().estimate()).toBe(82);
    setFolded(true);
    expect(geometry().estimate()).toBe(28);
    setFolded(false);
    expect(geometry().estimate()).toBe(82);
    dispose();
  });
});

it("reuses content versions on remount and detects edits made with no window mounted", () => {
  setDoc("byId", { root: row("root") });
  const readVersion = () => createRoot(dispose => {
    const model = createOutlineWindowModel(() => ["root"], () => (_id, _page, stored) => stored);
    const geometry = createOutlineWindowGeometry(model, () => 900, () => "16px sans-serif");
    const version = geometry().version();
    dispose();
    return version;
  });
  const initial = readVersion();
  expect(readVersion()).toBe(initial);
  setDoc("byId", "root", "raw", "edited while unmounted");
  expect(readVersion()).toBeGreaterThan(initial);
});
