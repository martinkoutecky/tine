import { afterEach, describe, expect, it, vi } from "vitest";

import { backend } from "../../backend";
import { captureBinding } from "../../binding";
import { pinPageWhileDrafting } from "../workingSet";
import { subscribeOwnerFlush } from "./wiring";

// CLI slice 2: before a command-line write reads a page, the owner asks the
// window to save the text typed on it (owner/window.rs). The window runs its
// page barrier and answers; a request for another binding gets no answer.

type Request = { id: number; page: string; binding_generation: number };

async function subscribe() {
  let deliver: (request: Request) => void = () => {};
  const b = backend() as unknown as Record<string, unknown>;
  const answer = vi.fn(async () => {});
  b.onOwnerFlushRequest = vi.fn(async (cb: (request: Request) => void) => { deliver = cb; return () => {}; });
  b.ownerFlushAnswer = answer;
  const stop = subscribeOwnerFlush();
  await vi.waitFor(() => expect(b.onOwnerFlushRequest).toHaveBeenCalled());
  return { emit: (request: Request) => deliver(request), answer, stop };
}

afterEach(() => {
  const b = backend() as unknown as Record<string, unknown>;
  delete b.onOwnerFlushRequest;
  delete b.ownerFlushAnswer;
});

describe("owner flush requests", () => {
  const generation = () => captureBinding().backendGeneration;

  it("answers saved when the page has no unsaved typed text", async () => {
    const { emit, answer, stop } = await subscribe();
    emit({ id: 7, page: "Projects", binding_generation: generation() });
    await vi.waitFor(() => expect(answer).toHaveBeenCalledWith(7, true, generation()));
    stop();
  });

  it("answers not saved while a component holds typed text outside the document", async () => {
    const release = pinPageWhileDrafting(() => "Composing");
    const { emit, answer, stop } = await subscribe();
    emit({ id: 8, page: "Composing", binding_generation: generation() });
    await vi.waitFor(() => expect(answer).toHaveBeenCalledWith(8, false, generation()));
    release();
    stop();
  });

  it("ignores a request for another graph binding", async () => {
    const { emit, answer, stop } = await subscribe();
    emit({ id: 3, page: "A", binding_generation: generation() + 1 });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(answer).not.toHaveBeenCalled();
    stop();
  });
});
