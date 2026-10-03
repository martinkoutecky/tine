import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { clearTransientLayersForTest } from "../transientLayers";
import {
  QUERY_TEXT_OPEN_KEY,
  QueryBuilder,
  resetQueryRegistryRevisionForTests,
  type BuilderSession,
} from "./QueryBuilder";
import { contentFilter, taskFilter } from "../editor/queryBuilder";
import type { Filter, ParsedQuery, Query, RegistrySnapshot } from "../editor/queryIr";

// **The live query-text pane** (SPEC §7.5, §4.3.1–§4.3.2, I-20, I-4/I-9).
//
// The pane is always there inside an open sheet, and everything it shows is an
// answer FROM RUST about text that is still on screen. The properties pinned
// here are the ones a slow or failing engine can break invisibly:
//
//  - **Both landing paths take the same gate.** A parse that succeeds for text
//    the user has replaced is stale; a parse that FAILS for it is worse, because
//    it reads as "your current text is broken" about text nobody has judged.
//  - **Save never writes an earlier good parse.** The button is enabled only for
//    a good parse OF THE CURRENT REVISION.
//  - **A refusal is shown, never swallowed** (I-4/I-9), and a rejection keeps the
//    last reading on screen, greyed, rather than reporting a typo as an empty
//    graph.
//  - **The options map is not edited here** (§4.3.1), and an opaque `og_options`
//    survives a text edit untouched.

const EMPTY_REGISTRY: RegistrySnapshot = { generation: 1, rows: [] };

function session(filter: Filter): BuilderSession {
  return { query: { anchor: "block", filter, source: { kind: "builder" } }, view: {} };
}

function parsedOf(filter: Filter, extra?: Partial<Query>): ParsedQuery {
  return {
    query: { anchor: "block", filter, source: { kind: "tql", original: "…" }, ...extra },
    view: {},
  };
}

function nested(depth: number, leaf: Filter): Filter {
  let filter = leaf;
  for (let level = 0; level < depth; level += 1) filter = { kind: "and", items: [filter] };
  return filter;
}

function mountBuilder(initial: BuilderSession) {
  const host = document.createElement("div");
  document.body.append(host);
  const [current, setCurrent] = createSignal<BuilderSession>(initial);
  const changes: BuilderSession[] = [];
  const dispose = render(
    () => (
      <QueryBuilder
        session={current}
        onChange={(next) => {
          changes.push(next);
          setCurrent(next);
        }}
      />
    ),
    host,
  );
  const open = (): HTMLElement => {
    const before = new Set(document.querySelectorAll<HTMLElement>(".qs-sheet"));
    host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
    const sheet = [...document.querySelectorAll<HTMLElement>(".qs-sheet")].find(
      (el) => !before.has(el),
    );
    if (!sheet) throw new Error("the sheet did not open");
    return sheet;
  };
  const close = () => host.querySelector<HTMLButtonElement>(".qs-gear")!.click();
  return { host, open, close, changes, session: current, setSession: setCurrent, dispose };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const settle = async () => {
  for (let i = 0; i < 10; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
};
/** Longer than `PANE_DEBOUNCE_MS`, plus the promise chain behind the parse. */
const settleParse = async () => {
  await wait(220);
  await settle();
};

const pane = () => document.querySelector<HTMLElement>(".query-text-pane")!;
const textarea = () => document.querySelector<HTMLTextAreaElement>(".query-text-pane-input")!;
const saveButton = () => document.querySelector<HTMLButtonElement>(".query-text-pane-save")!;
/** Everything the pane says went wrong, wherever it says it: the status line
 *  (a refusal, or a rejection with no diagnostics) plus the structured list. A
 *  message must appear in exactly one of them, never in both. */
const errorText = () =>
  [
    ...document.querySelectorAll(".query-text-pane-error"),
    ...document.querySelectorAll(".query-text-pane-diagnostic-message"),
  ]
    .map((el) => el.textContent)
    .join(" ");
const type = (value: string) => {
  const input = textarea();
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
};

/** A `parseQuery` mock whose answers are released by hand, so "a stale response
 *  lands while a newer one is pending" is a state the test can actually reach. */
function deferredParser() {
  const waiting = new Map<string, { resolve: (value: ParsedQuery) => void; reject: (error: unknown) => void }>();
  const spy = vi.spyOn(backend(), "parseQuery").mockImplementation(
    (text: string) =>
      new Promise<ParsedQuery>((resolve, reject) => {
        waiting.set(text, { resolve, reject });
      }),
  );
  return {
    spy,
    pending: () => [...waiting.keys()],
    resolve(text: string, value: ParsedQuery) {
      const entry = waiting.get(text);
      if (!entry) throw new Error(`nothing is waiting on a parse of ${JSON.stringify(text)}`);
      waiting.delete(text);
      entry.resolve(value);
    },
    reject(text: string, error: unknown) {
      const entry = waiting.get(text);
      if (!entry) throw new Error(`nothing is waiting on a parse of ${JSON.stringify(text)}`);
      waiting.delete(text);
      entry.reject(error);
    },
  };
}


beforeEach(() => {
  // GH #619 item 4: the text pane sits behind a remembered toggle; these tests are about the pane.
  localStorage.setItem(QUERY_TEXT_OPEN_KEY, "1");
  resetQueryRegistryRevisionForTests();
  vi.spyOn(backend(), "queryFacets").mockResolvedValue([]);
  vi.spyOn(backend(), "queryRegistry").mockResolvedValue(EMPTY_REGISTRY);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and task is TODO");
});

afterEach(() => {
  clearTransientLayersForTest();
  resetSharedQueryResultsForTests();
  resetQueryRegistryRevisionForTests();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("the pane is there, and it is the engine's answer", () => {
  it("is visible and editable inside an open sheet, and mounts nothing at rest", async () => {
    const printQuery = vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and task is TODO");
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      // At rest: one sentence, no pane, and nothing printed.
      expect(document.querySelector(".query-text-pane")).toBeNull();
      expect(printQuery).not.toHaveBeenCalled();

      builder.open();
      await settle();
      expect(pane()).toBeTruthy();
      // No `<details>` to discover: the control is open, and it holds the text
      // Rust printed rather than any the frontend spelled.
      expect(document.querySelector("details")).toBeNull();
      expect(textarea().disabled).toBe(false);
      expect(textarea().value).toBe("@block and task is TODO");
      expect(printQuery).toHaveBeenCalledTimes(1);
    } finally {
      builder.dispose();
    }
  });

  it("shows a print refusal instead of an empty box (I-4/I-9)", async () => {
    vi.spyOn(backend(), "printQuery").mockRejectedValue(new Error("this query cannot be said in the OG DSL"));
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      expect(errorText()).toContain("cannot be said in the OG DSL");
      expect(textarea().disabled).toBe(true);
    } finally {
      builder.dispose();
    }
  });
});

describe("I-20: only an answer about what is on screen right now", () => {
  it("drops a stale SUCCESS that lands while a newer edit is pending", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();

      type("@block and task is TODO");
      await settleParse();
      expect(parser.pending()).toContain("@block and task is TODO");

      // The user keeps typing before the first answer comes back.
      type("@block and task is DOING");
      await settleParse();

      // The FIRST parse now succeeds — for text nobody is looking at.
      parser.resolve("@block and task is TODO", parsedOf(taskFilter(["TODO"])));
      await settle();
      expect(saveButton().disabled).toBe(true);
      expect(builder.changes).toHaveLength(0);

      // The second one lands and is the one that counts.
      parser.resolve("@block and task is DOING", parsedOf(taskFilter(["DOING"])));
      await settle();
      expect(saveButton().disabled).toBe(false);
      saveButton().click();
      expect(builder.changes).toHaveLength(1);
      expect(JSON.stringify(builder.changes[0].query.filter)).toContain("DOING");
    } finally {
      builder.dispose();
    }
  });

  it("drops a stale REJECTION, so a superseded failure never accuses the current text", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();

      type("@block and taskk is TODO");
      await settleParse();
      type("@block and task is TODO");
      await settleParse();

      parser.reject("@block and taskk is TODO", new Error("unknown identifier `taskk`"));
      await settle();
      expect(errorText()).not.toContain("taskk");
      expect(textarea().getAttribute("aria-invalid")).toBeNull();

      parser.resolve("@block and task is TODO", parsedOf(taskFilter(["TODO"])));
      await settle();
      expect(saveButton().disabled).toBe(false);
    } finally {
      builder.dispose();
    }
  });

  it("drops an answer for a session that was replaced underneath the pane", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      type("@block and task is TODO");
      await settleParse();

      // A chip edit elsewhere, or a save landing: a NEW session object.
      builder.setSession(session(contentFilter("elsewhere")));
      await settle();

      parser.resolve("@block and task is TODO", parsedOf(taskFilter(["TODO"])));
      await settle();
      expect(saveButton().disabled).toBe(true);
      expect(builder.changes).toHaveLength(0);
    } finally {
      builder.dispose();
    }
  });

  it("drops an answer for a pane that was closed, and reopens clean", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      type("@block and task is BROKEN");
      await settleParse();

      builder.close();
      await settle();
      expect(document.querySelector(".query-text-pane")).toBeNull();

      // The in-flight answer lands into a pane that no longer exists. Nothing
      // is rendered and nothing is committed.
      parser.reject("@block and task is BROKEN", new Error("unknown marker `BROKEN`"));
      await settle();
      expect(builder.changes).toHaveLength(0);

      builder.open();
      await settle();
      // Reopened: the draft and the error are gone, and the text is the
      // engine's print of the query that is actually saved.
      expect(errorText()).toBe("");
      expect(textarea().value).toBe("@block and task is TODO");
      expect(saveButton().disabled).toBe(true);
    } finally {
      builder.dispose();
    }
  });
});

describe("invalid text is recoverable, not destructive", () => {
  it("keeps the rows on screen greyed while the text is invalid, and clears on repair", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      const sheet = builder.open();
      await settle();
      const rowsBefore = sheet.querySelectorAll(".qs-row").length;
      expect(rowsBefore).toBeGreaterThan(0);

      type("@block and task is");
      await settleParse();
      parser.resolve(
        "@block and task is",
        parsedOf(taskFilter([]), {
          diagnostics: [{ message: "a task marker is missing", kind: "syntax", span: { start: 17, end: 17 } }],
        }),
      );
      await settle();

      expect(errorText()).toContain("a task marker is missing");
      // Once, not twice: the structured item carries it and the status line
      // stands down.
      expect(document.querySelectorAll(".query-text-pane-error")).toHaveLength(0);
      expect(saveButton().disabled).toBe(true);
      // The last reading that RAN is still drawn — blanking it would report a
      // typo as an empty graph.
      expect(sheet.querySelectorAll(".qs-row").length).toBe(rowsBefore);
      // The sheet says so, and the host block greys its results the same way
      // (`query-stale`, pinned by `QueryMacro.ir.test.tsx`).
      expect(sheet.classList.contains("qs-sheet-stale")).toBe(true);

      type("@block and task is TODO");
      await settleParse();
      parser.resolve("@block and task is TODO", parsedOf(taskFilter(["TODO"])));
      await settle();
      expect(errorText()).toBe("");
      expect(sheet.classList.contains("qs-sheet-stale")).toBe(false);
    } finally {
      builder.dispose();
    }
  });

  // §4.3.1. The pane edits the FORM. An options map pasted into it is a
  // different control's business, and the refusal says which one.
  it("refuses to be an options editor and names the control that is one", async () => {
    const parse = vi.spyOn(backend(), "parseQuery");
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      type('@block and task is TODO {:title "Inbox"}');
      await settleParse();
      expect(errorText()).toContain("options map");
      expect(errorText()).toContain("Display");
      expect(parse).not.toHaveBeenCalled();
      expect(saveButton().disabled).toBe(true);
    } finally {
      builder.dispose();
    }
  });

  // §4.3.1 carry-forward: an absent map in pane text never means "delete the
  // title", and the view the reader chose is not a casualty of a text edit.
  it("carries the view and the opaque options map through a text save", async () => {
    const parser = deferredParser();
    const start: BuilderSession = {
      query: {
        anchor: "block",
        filter: taskFilter(["TODO"]),
        source: { kind: "tql", original: "@block and task is TODO", og_options: '{:title "Inbox" :collapsed? true}' },
      },
      view: { group_by: "page" },
    };
    const builder = mountBuilder(start);
    try {
      builder.open();
      await settle();
      type("@block and task is DOING");
      await settleParse();
      // The engine's answer carries NO options map — the pane never sees one.
      parser.resolve(
        "@block and task is DOING",
        { query: { anchor: "block", filter: taskFilter(["DOING"]), source: { kind: "tql", original: "@block and task is DOING" } }, view: {} },
      );
      await settle();
      saveButton().click();

      const saved = builder.changes.at(-1)!;
      expect(JSON.stringify(saved.query.filter)).toContain("DOING");
      expect(saved.query.source).toEqual({
        kind: "tql",
        original: "@block and task is DOING",
        og_options: '{:title "Inbox" :collapsed? true}',
      });
      expect(saved.view).toEqual({ group_by: "page" });
    } finally {
      builder.dispose();
    }
  });
});

describe("§4.3.2: the diagnostic's own structure survives to the UI", () => {
  it("renders each diagnostic's message, kind, disabled state and the engine's own suggestions", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      type("@block and stauts is active and (off deadline)");
      await settleParse();
      parser.resolve(
        "@block and stauts is active and (off deadline)",
        parsedOf(taskFilter(["TODO"]), {
          diagnostics: [
            {
              message: "`stauts` is not a property in this graph",
              kind: "unknown_ident",
              span: { start: 11, end: 17 },
              suggestions: ["Tine knows: prop('status')"],
            },
            { message: "a deadline needs a date", kind: "syntax", disabled: true, span: { start: 26, end: 45 } },
          ],
        }),
      );
      await settle();

      const items = [...document.querySelectorAll<HTMLElement>(".query-text-pane-diagnostic")];
      expect(items).toHaveLength(2);
      expect(items[0].getAttribute("data-kind")).toBe("unknown_ident");
      expect(items[0].textContent).toContain("is not a property in this graph");
      expect(items[0].textContent?.match(/Tine knows:/g)).toHaveLength(1);
      expect(items[0].textContent).toContain("status");
      expect(items[0].querySelector(".query-text-pane-locate")).toBeTruthy();

      expect(items[1].getAttribute("data-kind")).toBe("syntax");
      expect(items[1].classList.contains("is-disabled")).toBe(true);
      expect(items[1].textContent).toContain("the query still runs");
      // A disabled-only diagnostic does not invalidate (§3.5); this parse has a
      // live one too, so it does.
      expect(saveButton().disabled).toBe(true);
    } finally {
      builder.dispose();
    }
  });

  // Spans are UTF-16 code units, converted ONCE at the Rust boundary precisely
  // because the consumer is JavaScript. Converting again here would move every
  // offset past the first non-ASCII character — an astral emoji is two code
  // units, so a span computed in code points would land in the wrong word.
  it("selects the exact span in text with astral characters, without re-converting", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      const source = '@block and content ~ "🎉🎉" and stauts is x';
      const start = source.indexOf("stauts");
      const end = start + "stauts".length;
      type(source);
      await settleParse();
      parser.resolve(
        source,
        parsedOf(taskFilter(["TODO"]), {
          diagnostics: [{ message: "unknown identifier", kind: "unknown_ident", span: { start, end } }],
        }),
      );
      await settle();

      document.querySelector<HTMLButtonElement>(".query-text-pane-locate")!.click();
      const input = textarea();
      expect(input.selectionStart).toBe(start);
      expect(input.selectionEnd).toBe(end);
      expect(input.value.slice(input.selectionStart!, input.selectionEnd!)).toBe("stauts");
    } finally {
      builder.dispose();
    }
  });

  it("offers no span jump for a diagnostic about text that has since changed", async () => {
    const parser = deferredParser();
    const builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      type("@block and stauts is x");
      await settleParse();
      parser.resolve(
        "@block and stauts is x",
        parsedOf(taskFilter(["TODO"]), {
          diagnostics: [{ message: "unknown identifier", kind: "unknown_ident", span: { start: 11, end: 17 } }],
        }),
      );
      await settle();
      expect(document.querySelector(".query-text-pane-locate")).toBeTruthy();

      // One more keystroke: the offsets are about a text nobody is editing now.
      type("@block and stauts is xy");
      await settle();
      expect(document.querySelector(".query-text-pane-locate")).toBeNull();
    } finally {
      builder.dispose();
    }
  });
});

describe("§7.8: the part the sheet cannot draw is still reachable", () => {
  it("opens and focuses the pane from the ⟨advanced⟩ chip without touching the IR", async () => {
    const builder = mountBuilder(session(nested(64, taskFilter(["TODO"]))));
    try {
      const sheet = builder.open();
      await settle();
      const before = JSON.stringify(builder.session());

      const chip = sheet.querySelector<HTMLButtonElement>(".qs-advanced-open")!;
      expect(chip).toBeTruthy();
      expect(chip.tagName).toBe("BUTTON");
      expect(chip.getAttribute("aria-label")).toContain("Edit in the query text");

      chip.click();
      await settle();
      expect(document.activeElement).toBe(textarea());
      expect(JSON.stringify(builder.session())).toBe(before);
      expect(builder.changes).toHaveLength(0);
    } finally {
      builder.dispose();
    }
  });
});

describe("GH #619 item 4: the text is behind an \"Edit as text\" toggle that remembers its state", () => {
  const toggle = () => document.querySelector<HTMLButtonElement>(".qs-text-toggle")!;

  it("opens the sheet without the text, shows it on the toggle, and remembers the choice", async () => {
    localStorage.removeItem(QUERY_TEXT_OPEN_KEY);
    const printQuery = vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and task is TODO");
    let builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      // Default: the rows are the editor; no text box, nothing printed.
      expect(document.querySelector(".query-text-pane")).toBeNull();
      expect(toggle().textContent).toBe("Edit as text");
      expect(toggle().getAttribute("aria-expanded")).toBe("false");
      expect(printQuery).not.toHaveBeenCalled();

      toggle().click();
      await settle();
      expect(toggle().getAttribute("aria-expanded")).toBe("true");
      expect(textarea().value).toBe("@block and task is TODO");
      builder.close();
    } finally {
      builder.dispose();
    }

    // A later sheet (even a fresh mount) opens the way it was left.
    builder = mountBuilder(session(taskFilter(["TODO"])));
    try {
      builder.open();
      await settle();
      expect(textarea().value).toBe("@block and task is TODO");
      toggle().click();
      await settle();
      expect(document.querySelector(".query-text-pane")).toBeNull();
    } finally {
      builder.dispose();
    }
    const again = mountBuilder(session(taskFilter(["TODO"])));
    try {
      again.open();
      await settle();
      expect(document.querySelector(".query-text-pane")).toBeNull();
    } finally {
      again.dispose();
    }
  });

  it("still opens a retained leaf for editing as text from a closed toggle", async () => {
    localStorage.removeItem(QUERY_TEXT_OPEN_KEY);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("@block and (foo bar)");
    const builder = mountBuilder(session({ kind: "raw", text: "foo bar", diagnostic_kind: "syntax" }));
    try {
      builder.open();
      await settle();
      expect(document.querySelector(".query-text-pane")).toBeNull();
      const edit = document.querySelector<HTMLButtonElement>(".qs-raw-edit");
      expect(edit).not.toBeNull();
      edit!.click();
      await settle();
      expect(pane()).toBeTruthy();
      expect(document.activeElement).toBe(textarea());
    } finally {
      builder.dispose();
    }
  });
});
