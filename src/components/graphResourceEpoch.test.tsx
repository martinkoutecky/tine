import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpGraphEpoch } from "../graphSession";
import { setAudioPlayer } from "../ui";
import { JournalTemplateField } from "./JournalTemplateField";
import { ConflictFileRow } from "./JournalConflictFileRow";
import { AudioOverlay } from "./AudioOverlay";
import { QueryBuilder } from "./QueryBuilder";

// UI-GRAPH-RESOURCE-OUTLIVES-EPOCH-1112. Each view below reads through
// `graphOwner`, which retires the read when the graph epoch moves (graph open
// or switch, a rename, typography, journal-title format). The reply that was
// outstanding at that moment must not stay on screen as the final answer: the
// view asks again and shows what the graph holds now.

afterEach(() => { setAudioPlayer(null); document.body.innerHTML = ""; vi.restoreAllMocks(); });

function held<T>() {
  let reply!: (value: T) => void;
  const promise = new Promise<T>((resolve) => { reply = resolve; });
  return { promise, reply: (value: T) => reply(value) };
}
function mount(view: () => any) {
  const root = document.createElement("div"); document.body.append(root);
  return { root, dispose: render(view, root) };
}

it("Settings lists the journal templates after the epoch moves under the first read", async () => {
  const first = held<string[]>();
  const list = vi.spyOn(backend(), "listTemplateNames")
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue(["Daily"]);
  const { root, dispose } = mount(() => <JournalTemplateField />);
  try {
    await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    bumpGraphEpoch();
    first.reply(["Daily"]);
    await vi.waitFor(() => expect([...root.querySelectorAll("option")].map((o) => o.textContent)).toContain("Daily"));
    expect(list).toHaveBeenCalledTimes(2);
  } finally { dispose(); }
});

it("a duplicate journal file's preview shows its text, not \"empty file\", after the epoch moves under the read", async () => {
  const first = held<string>();
  const read = vi.spyOn(backend(), "readJournalFile")
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue("- the stray day's notes");
  const file = { name: "2026_11_12.md", path: "journals/2026_11_12.md", preview: "", canonical: false };
  const { root, dispose } = mount(() => <ConflictFileRow file={file} onOpen={() => {}} onRename={() => {}} onTrash={() => {}} />);
  try {
    root.querySelector<HTMLButtonElement>("button.settings-asset-name")!.click();
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    bumpGraphEpoch();
    first.reply("- the stray day's notes");
    await vi.waitFor(() => expect(root.querySelector("pre.journal-conflict-content")?.textContent).toBe("- the stray day's notes"));
  } finally { dispose(); }
});

it("the audio player gets its stream after the epoch moves under the first stream request", async () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(() => Promise.resolve());
  const first = held<string>();
  const stream = vi.spyOn(backend(), "streamAsset")
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue("stream:track");
  setAudioPlayer({ url: "../assets/track.mp3", name: "track" });
  const { root, dispose } = mount(() => <AudioOverlay />);
  try {
    await vi.waitFor(() => expect(stream).toHaveBeenCalledTimes(1));
    bumpGraphEpoch();
    first.reply("stream:track");
    await vi.waitFor(() => expect(root.querySelector("audio")?.getAttribute("src")).toBe("stream:track"));
  } finally { dispose(); }
});

it("the query builder's text pane shows the query after the epoch moves under its print", async () => {
  const first = held<string>();
  const print = vi.spyOn(backend(), "printQuery")
    .mockImplementationOnce(() => first.promise)
    .mockResolvedValue("@block and #todo");
  const query = { anchor: "block" as const, filter: { kind: "true" as const }, source: { kind: "builder" as const } };
  const session = { query, view: {} };
  const { root, dispose } = mount(() => <QueryBuilder sheetAlwaysOpen session={() => session} onChange={() => {}} />);
  try {
    root.querySelector<HTMLButtonElement>(".qs-text-toggle")!.click();
    await vi.waitFor(() => expect(print).toHaveBeenCalledTimes(1));
    bumpGraphEpoch();
    first.reply("@block and #todo");
    await vi.waitFor(() => expect(root.querySelector<HTMLTextAreaElement>(".query-text-pane-input")?.value).toBe("@block and #todo"));
  } finally { dispose(); }
});
