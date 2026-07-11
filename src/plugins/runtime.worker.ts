/// <reference lib="webworker" />

import { instantiatePluginGuest, invokePluginGuest, type PluginGuest, type PluginGuestLimits } from "./guest";
import type { PluginEvent } from "./protocol";

type WorkerRequest =
  | { id: number; kind: "init"; wasm: ArrayBuffer; limits: PluginGuestLimits }
  | { id: number; kind: "invoke"; event: PluginEvent }
  | { id: number; kind: "dispose" };

type WorkerReply =
  | { id: number; ok: true; result?: unknown }
  | { id: number; ok: false; error: string };

let guest: PluginGuest | null = null;

function reply(message: WorkerReply) {
  self.postMessage(message);
}

self.onmessage = async (message: MessageEvent<WorkerRequest>) => {
  const request = message.data;
  try {
    switch (request.kind) {
      case "init":
        if (guest) throw new Error("guest is already initialized");
        guest = await instantiatePluginGuest(request.wasm, request.limits);
        reply({ id: request.id, ok: true });
        break;
      case "invoke":
        if (!guest) throw new Error("guest is not initialized");
        reply({ id: request.id, ok: true, result: invokePluginGuest(guest, request.event) });
        break;
      case "dispose":
        guest = null;
        reply({ id: request.id, ok: true });
        self.close();
        break;
    }
  } catch (error) {
    reply({ id: request.id, ok: false, error: error instanceof Error ? error.message : "plugin runtime failed" });
  }
};
