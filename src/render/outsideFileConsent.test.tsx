import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";
import { LinkSchemePrompt } from "../components/LinkSchemePrompt";
import { openOutboundLink } from "../components/ExternalLink";
import { FileConsentAsk, fileConsentAsk } from "../fileConsent";
import { endCaptureShow } from "../captureShow";
import { graphMeta, setGraphMeta } from "../graphSession";
import { acquirePdfDocument } from "../components/pdfDocumentShare";
import { ExternalPdfLink } from "./ExternalPdfLink";
import * as panes from "../panes";
import bidi from "../../tests/fixtures/outside-file-ask-bidi.json";

// tine#985: a file: link whose target is outside the graph asks first, naming
// the file; Cancel opens nothing, Open opens once; a late answer after the
// owner went away shows no prompt and opens nothing.

beforeAll(initParser);
const disposers: (() => void)[] = [];
beforeEach(() => {
  const host = document.createElement("div"); document.body.append(host);
  disposers.push(render(() => <LinkSchemePrompt />, host));
});
afterEach(() => {
  disposers.splice(0).forEach(dispose => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
  setToasts([]);
  setGraphMeta(null);
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const ask = (display: string, lossy = false) => new FileConsentAsk(display, lossy);
const prompt = () => document.querySelector<HTMLFormElement>(".link-scheme-prompt");
const cancel = () => prompt()!.querySelector<HTMLButtonElement>('button[type="button"]')!.click();

function clickLink(source: string): void {
  const host = document.createElement("div"); document.body.append(host);
  disposers.push(render(() => <InlineText text={source} />, host));
  host.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
}

/** First call answers Ask, later calls succeed; records every call. */
function askThenOpen(display: string) {
  return vi.spyOn(backend(), "openExternal").mockImplementation(async (_url, _scheme, file) => {
    if (file?.purpose === "link") throw ask(display);
  });
}

describe("note links (P1)", () => {
  it("asks naming the file, and Open makes one agreed call with the click's generation", async () => {
    const open = askThenOpen("/T/s.txt");
    const generation = backend().graphBindingGeneration();
    clickLink("[s](file:///T/s.txt)");
    await flush();
    expect(prompt()!.textContent).toContain("/T/s.txt");
    expect(prompt()!.textContent).toContain("outside this graph");
    prompt()!.requestSubmit();
    await flush();
    expect(open.mock.calls.map(call => call[2])).toEqual([
      { purpose: "link", bindingGeneration: generation, show: undefined },
      { purpose: "link-agreed", bindingGeneration: generation, show: undefined },
    ]);
    expect(toasts()).toEqual([]);
  });

  it("Cancel opens nothing; ticking allow-for-this-graph sends the remember purpose", async () => {
    const open = askThenOpen("/T/s.txt");
    clickLink("[s](file:///T/s.txt)");
    await flush();
    cancel();
    await flush();
    expect(open).toHaveBeenCalledTimes(1);
    clickLink("[s](file:///T/s.txt)");
    await flush();
    prompt()!.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    prompt()!.requestSubmit();
    await flush();
    expect(open.mock.calls.map(call => call[2]?.purpose)).toEqual(["link", "link", "link-agreed-remember"]);
  });

  it("an inside file opens on the first call with no prompt", async () => {
    const open = vi.spyOn(backend(), "openExternal").mockResolvedValue();
    clickLink("[x](file:///G/files/x.txt)");
    await flush();
    expect(prompt()).toBeNull();
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("a first reply after a graph switch shows no prompt, also from the caret route", async () => {
    for (const graphScoped of [true, false]) {
      let answer!: (error: unknown) => void;
      vi.spyOn(backend(), "openExternal").mockImplementation(() => new Promise((_, reject) => { answer = reject; }));
      const pending = openOutboundLink("file:///T/s.txt", graphScoped);
      invalidateBinding();
      answer(ask("/T/s.txt"));
      await pending;
      expect(prompt()).toBeNull();
      expect(toasts()).toEqual([]);
      vi.restoreAllMocks();
    }
  });

  it("a prompt open during a graph switch is cancelled and opens nothing", async () => {
    const open = askThenOpen("/T/s.txt");
    const pending = openOutboundLink("file:///T/s.txt");
    await flush();
    expect(prompt()).not.toBeNull();
    invalidateBinding();
    await pending;
    expect(prompt()).toBeNull();
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("shows the unreadable-name note for a lossy display", async () => {
    askThenOpen("/T/n�.txt");
    vi.spyOn(backend(), "openExternal").mockRejectedValueOnce(ask("/T/n�.txt", true));
    const pending = openOutboundLink("file:///T/n%FF.txt");
    await flush();
    expect(prompt()!.textContent).toContain("can't be shown");
    cancel(); await pending;
  });

  it("a right-to-left override in the name shows as an escape in path order, with the unreadable note", async () => {
    // The wire is native's Ask for this path (outside_file_tests pins it).
    vi.spyOn(backend(), "openExternal").mockImplementation(async (_url, _scheme, file) => {
      if (file?.purpose === "link") throw fileConsentAsk(new Error(bidi.wire));
    });
    const pending = openOutboundLink("file:///T/invoice%E2%80%AEfdp.exe");
    await flush();
    const text = prompt()!.textContent!;
    expect(text).toContain("/T/invoice<U+202E>fdp.exe");
    expect(text).not.toMatch(/[\u202a-\u202e\u2066-\u2069\u200e\u200f\u061c]/);
    expect(text).toContain("can't be shown");
    cancel(); await pending;
  });
});

describe("Quick Capture shows", () => {
  afterEach(() => { endCaptureShow(); });

  /** Native's show check: a call naming a show must name the current one; a
   * first call is admitted in the current show and its Ask names it. */
  function nativeCapture(display: string) {
    const native = { show: 41 };
    const open = vi.spyOn(backend(), "openExternal").mockImplementation(async (_url, _scheme, file) => {
      if (file?.show !== undefined && file.show !== native.show) throw new Error("stale-graph-binding");
      if (file?.purpose === "link") throw new FileConsentAsk(display, false, native.show);
      if (file?.show !== native.show) throw new Error("stale-graph-binding");
    });
    return { native, open };
  }

  it("Open carries the show the Ask named, never offers to remember, and a hide cancels the prompt", async () => {
    endCaptureShow();
    const { open } = nativeCapture("/G/files/x.txt");
    const opened = openOutboundLink("file:///G/files/x.txt", false);
    await flush();
    expect(prompt()!.querySelector('input[type="checkbox"]')).toBeNull();
    prompt()!.requestSubmit();
    await opened;
    expect(open.mock.calls.map(call => call[2]?.show)).toEqual([undefined, 41]);
    const pending = openOutboundLink("file:///G/files/x.txt", false);
    await flush();
    endCaptureShow(); // hide
    await pending;
    expect(prompt()).toBeNull();
    expect(open).toHaveBeenCalledTimes(3);
    expect(toasts()).toEqual([]);
  });

  it("a click in show B prompts and opens in B when capture-shown was missed", async () => {
    endCaptureShow(); // show A
    const { native, open } = nativeCapture("/T/s.txt");
    endCaptureShow(); // hide A
    native.show = 42; // show B; its capture-shown never arrives
    const pending = openOutboundLink("file:///T/s.txt", false);
    await flush();
    expect(prompt()).not.toBeNull();
    prompt()!.requestSubmit();
    await pending;
    expect(open.mock.calls.map(call => call[2]?.show)).toEqual([undefined, 42]);
    expect(toasts()).toEqual([]);
  });

  it("show A's late first reply in show B shows no prompt", async () => {
    endCaptureShow();
    let answer!: (error: unknown) => void;
    vi.spyOn(backend(), "openExternal").mockImplementation(() => new Promise((_, reject) => { answer = reject; }));
    const pending = openOutboundLink("file:///T/s.txt", false);
    endCaptureShow(); // capture-shown for B
    answer(ask("/T/s.txt"));
    await pending;
    expect(prompt()).toBeNull();
  });
});

describe("rendered graph-relative links (P3)", () => {
  it("asks for a path that leaves the graph, then repeats the same path and generation", async () => {
    setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
    const generation = backend().graphBindingGeneration();
    const open = vi.spyOn(backend(), "openGraphFile").mockImplementation(async (_path, _generation, purpose) => {
      if (purpose === "link") throw ask("/home/user/T/back/x.txt");
    });
    clickLink("[x](out/back/x.txt)");
    await flush();
    expect(prompt()!.textContent).toContain("/home/user/T/back/x.txt");
    prompt()!.requestSubmit();
    await flush();
    expect(open.mock.calls).toEqual([
      ["out/back/x.txt", generation, "link"],
      ["out/back/x.txt", generation, "link-agreed"],
    ]);
    expect(toasts()).toEqual([]);
    clickLink("[x](out/back/x.txt)");
    await flush();
    cancel();
    await flush();
    expect(open).toHaveBeenCalledTimes(3);
  });
});

describe("outside PDFs (P2): the pending document owns the prompt", () => {
  function clickPdf() {
    const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
    const host = document.createElement("div"); document.body.append(host);
    disposers.push(render(() => <ExternalPdfLink dest="file:///T/s.pdf" path="/T/s.pdf" label="s" />, host));
    host.querySelector("a")!.click();
    return open.mock.calls[0][4]!.externalRead!;
  }

  it("closing the last view while the prompt is up cancels it and reads nothing", async () => {
    const api = backend(), original = api.readExternalPdf;
    const read = vi.fn(async (_url: string, _generation: number, purpose: string) => {
      if (purpose === "link") throw ask("/T/s.pdf");
      return new Uint8Array([1]);
    });
    api.readExternalPdf = read;
    try {
      const externalRead = clickPdf();
      let failure: unknown;
      const held = acquirePdfDocument("file:///T/s.pdf#lifetime", async (adopt) => {
        try { await externalRead(adopt.signal); } catch (error) { failure = error; }
        return { kind: "error", message: "done" };
      });
      await flush();
      expect(prompt()).not.toBeNull();
      held.release();
      await held.loaded;
      expect(prompt()).toBeNull();
      expect(read).toHaveBeenCalledTimes(1);
      expect(String(failure)).toContain("external-pdf-not-opened");
    } finally { if (original) api.readExternalPdf = original; else delete api.readExternalPdf; }
  });

  it("closing it before the Ask arrives shows no prompt", async () => {
    const api = backend(), original = api.readExternalPdf;
    let answer!: (error: unknown) => void;
    const read = vi.fn(() => new Promise<Uint8Array>((_, reject) => { answer = reject; }));
    api.readExternalPdf = read;
    try {
      const externalRead = clickPdf();
      const held = acquirePdfDocument("file:///T/s.pdf#early", async (adopt) => {
        await externalRead(adopt.signal).catch(() => {});
        return { kind: "error", message: "done" };
      });
      await flush();
      held.release();
      answer(ask("/T/s.pdf"));
      await held.loaded;
      expect(prompt()).toBeNull();
      expect(read).toHaveBeenCalledTimes(1);
    } finally { if (original) api.readExternalPdf = original; else delete api.readExternalPdf; }
  });

  it("Cancel leaves the pane with Not opened; Open reads once more", async () => {
    const api = backend(), original = api.readExternalPdf;
    const read = vi.fn(async (_url: string, _generation: number, purpose: string) => {
      if (purpose === "link") throw ask("/T/s.pdf");
      return new Uint8Array([37]);
    });
    api.readExternalPdf = read;
    try {
      const signal = new AbortController().signal;
      const cancelled = clickPdf()(signal);
      await flush(); cancel();
      await expect(cancelled).rejects.toThrow("external-pdf-not-opened");
      const opened = clickPdf()(signal);
      await flush(); prompt()!.requestSubmit();
      expect(await opened).toEqual(new Uint8Array([37]));
      expect(read.mock.calls.map(call => call[2])).toEqual(["link", "link", "link-agreed"]);
    } finally { if (original) api.readExternalPdf = original; else delete api.readExternalPdf; }
  });
});
