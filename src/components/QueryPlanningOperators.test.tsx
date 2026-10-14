import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { clearTransientLayersForTest } from "../transientLayers";

afterEach(() => { clearTransientLayersForTest(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it.each(["scheduled", "deadline"] as const)("%s comparisons offer comparison operators and honest negation", async attr => {
  vi.spyOn(backend(), "printQuery").mockResolvedValue(`@block and ${attr} > +5d`);
  const [session, setSession] = createSignal<BuilderSession>({ query: { anchor: "block", filter: {
    kind: "leaf", leaf: { kind: "attr", attr, op: "gt", value: { kind: "date", literal: "+5d" } },
  }, source: { kind: "builder" } }, view: {} });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <QueryBuilder sheetAlwaysOpen session={session} onChange={setSession} />, root);
  const open = () => {
    root.querySelector<HTMLButtonElement>(".qs-op")!.click();
    return [...document.querySelectorAll<HTMLButtonElement>(".qs-option")];
  };
  try {
    const options = open();
    expect(options.map(option => option.textContent)).not.toContain("is not set");
    expect(options.map(option => option.textContent)).toContain("<");
    options.find(option => option.textContent === "<")!.click();
    expect(session().query.filter).toEqual({ kind: "and", items: [{ kind: "leaf", leaf: { kind: "attr", attr, op: "lt", value: { kind: "date", literal: "+5d" } } }] });
    open().find(option => option.textContent === "not <")!.click();
    expect(session().query.filter).toEqual({ kind: "and", items: [{ kind: "not", inner: { kind: "leaf", leaf: { kind: "attr", attr, op: "lt", value: { kind: "date", literal: "+5d" } } } }] });
  } finally { dispose(); }
});
