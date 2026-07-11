import { DEFAULT_GUEST_LIMITS, type PluginGuestLimits } from "./guest";
import type { PluginEvent, PluginResponse } from "./protocol";

interface WorkerLike {
  onmessage: ((event: MessageEvent<WorkerReply>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(message: unknown, transfer?: Transferable[]): void;
  terminate(): void;
}

type WorkerReply =
  | { id: number; ok: true; result?: unknown }
  | { id: number; ok: false; error: string };

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

export interface PluginRuntimeOptions {
  limits?: PluginGuestLimits;
  initializationTimeoutMs?: number;
  invocationTimeoutMs?: number;
  workerFactory?: () => WorkerLike;
}

export class PluginRuntimeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginRuntimeError";
  }
}

function defaultWorker(): WorkerLike {
  return new Worker(new URL("./runtime.worker.ts", import.meta.url), {
    type: "module",
    name: "tine-plugin-guest",
  });
}

/**
 * Owns one isolated plugin worker. A timeout is fatal: termination is the only
 * reliable way to stop an untrusted WebAssembly loop, so callers must recreate
 * the runtime before invoking that plugin again.
 */
export class PluginRuntime {
  private readonly worker: WorkerLike;
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;
  private dead = false;

  private constructor(worker: WorkerLike, private readonly invocationTimeoutMs: number) {
    this.worker = worker;
    worker.onmessage = (event) => this.receive(event.data);
    worker.onerror = () => this.failAll(new PluginRuntimeError("plugin worker crashed"));
  }

  static async create(wasm: ArrayBuffer, options: PluginRuntimeOptions = {}): Promise<PluginRuntime> {
    const runtime = new PluginRuntime((options.workerFactory ?? defaultWorker)(), options.invocationTimeoutMs ?? 250);
    const ownedBytes = wasm.slice(0);
    try {
      await runtime.request(
        { kind: "init", wasm: ownedBytes, limits: options.limits ?? DEFAULT_GUEST_LIMITS },
        options.initializationTimeoutMs ?? 2_000,
        [ownedBytes]
      );
      return runtime;
    } catch (error) {
      runtime.terminate();
      throw error;
    }
  }

  async invoke(event: PluginEvent): Promise<PluginResponse> {
    if (this.dead) throw new PluginRuntimeError("plugin runtime is unavailable");
    return (await this.request({ kind: "invoke", event }, this.invocationTimeoutMs)) as PluginResponse;
  }

  dispose() {
    if (this.dead) return;
    this.terminate();
  }

  private request(
    body: Record<string, unknown>,
    timeoutMs: number,
    transfer?: Transferable[]
  ): Promise<unknown> {
    if (this.dead) return Promise.reject(new PluginRuntimeError("plugin runtime is unavailable"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        const error = new PluginRuntimeError("plugin exceeded its execution time limit");
        reject(error);
        this.failAll(error);
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.worker.postMessage({ id, ...body }, transfer);
    });
  }

  private receive(reply: WorkerReply) {
    const pending = this.pending.get(reply.id);
    if (!pending) return;
    this.pending.delete(reply.id);
    clearTimeout(pending.timer);
    if (reply.ok) pending.resolve(reply.result);
    else pending.reject(new PluginRuntimeError(reply.error));
  }

  private failAll(error: PluginRuntimeError) {
    if (this.dead) return;
    this.dead = true;
    this.worker.terminate();
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  private terminate() {
    this.failAll(new PluginRuntimeError("plugin runtime was disposed"));
  }
}
