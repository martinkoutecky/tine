import { BrowserCoreLoadError } from "./browserCoreError";
import { PublishedQueryLimitError, type GraphRead, type GraphReply, type GraphRequest } from "./publishedGraphRpc";
import { QuickShareError } from "./quickShare/quickShare";
import { documentOf, listen, mainWindow } from "./windowRealm";

/** Open the document with one transferred buffer and one round trip. The worker
 * owns the graph for the life of the document and rejects all reads on failure. */
export async function openPublishedGraphWorker(bytes: Uint8Array<ArrayBuffer>, isShare: boolean) {
  let worker: Worker;
  try {
    worker = new Worker(new URL("./publishedGraph.worker.ts", import.meta.url), { type: "module" });
  } catch (error) {
    throw new BrowserCoreLoadError(error);
  }
  const pending = new Map<number, { resolve: (json: string) => void; reject: (error: Error) => void }>();
  const ownerDocument = documentOf(mainWindow);
  const lifetime = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let showFailure!: (error: BrowserCoreLoadError) => void;
  const failure = new Promise<BrowserCoreLoadError>(resolve => { showFailure = resolve; });
  let stopped: Error | undefined;
  let nextId = 0;
  let lastReply = 0;

  function stop(error: Error) {
    if (stopped) return;
    stopped = error;
    lifetime.abort();
    clearTimeout(timer);
    worker.terminate();
    for (const { reject } of pending.values()) reject(error);
    pending.clear();
  }

  function fail(cause: unknown) {
    if (stopped) return;
    const error = new BrowserCoreLoadError(cause);
    stop(error);
    showFailure(error);
  }

  function leave(event: PageTransitionEvent) {
    if (!event.persisted) stop(new Error("Publication closed"));
  }

  worker.onerror = (event) => {
    event.preventDefault();
    fail(event.message || "Worker failed");
  };
  worker.onmessageerror = () => fail("Could not receive a browser graph reply");
  worker.onmessage = ({ data }: MessageEvent<GraphReply>) => {
    lastReply = data.id;
    if ("error" in data && data.fatal) {
      fail(data.error);
      return;
    }
    const request = pending.get(data.id);
    if (!request) return;
    pending.delete(data.id);
    if ("json" in data) request.resolve(data.json);
    else request.reject(data.queryLimit ? new PublishedQueryLimitError(data.error)
      : data.shareReason ? new QuickShareError(data.shareReason, data.error) : new Error(data.error));
  };

  function request(method: GraphRequest["method"], args: GraphRequest["args"], transfer: Transferable[] = [], queryLocation?: string) {
    if (stopped) return Promise.reject(stopped);
    const id = ++nextId;
    return new Promise<string>((resolve, reject) => {
      pending.set(id, { resolve, reject });
      try {
        worker.postMessage({ id, method, args, queryLocation }, transfer);
      } catch (error) {
        fail(error);
      }
    });
  }

  // Plain HTTP exports lack Web Locks. Allow for queued graph reads and time
  // spent suspended before treating silence in a visible tab as a failure.
  function watchWithoutLock() {
    const interval = 30_000;
    let pulsePending = false;
    let missed = 0;
    function check() {
      if (stopped) return;
      const before = performance.now();
      const previousReply = lastReply;
      timer = setTimeout(() => {
        if (ownerDocument?.hidden || performance.now() - before > 2 * interval || lastReply !== previousReply) missed = 0;
        else if (!pulsePending) {
          pulsePending = true;
          void request("ping", []).then(() => { pulsePending = false; missed = 0; }, fail);
        } else if (++missed === 5) {
          fail("Worker stopped responding");
          return;
        }
        check();
      }, interval);
    }
    check();
  }

  try {
    if (ownerDocument) listen(ownerDocument, "pagehide", leave, { signal: lifetime.signal });
    const locks = globalThis.navigator?.locks;
    const lock = locks && `tine-publication-${crypto.randomUUID()}`;
    watchWithoutLock();
    const name: string = JSON.parse(await request("open", [bytes.buffer, isShare, lock], [bytes.buffer]));
    if (locks && lock) {
      // The worker acquires this lock before confirming open. When its realm
      // exits, the browser releases the lock and wakes us even if idle.
      clearTimeout(timer);
      void locks.request(lock, { signal: lifetime.signal }, () => fail("Worker closed"))
        .catch(error => { if (!lifetime.signal.aborted) fail(error); });
    }
    const read: GraphRead = ([method, ...args], queryLocation) => request(method, args, [], queryLocation);
    return { name, read, failure };
  } catch (error) {
    stop(error instanceof Error ? error : new Error(String(error)));
    throw error;
  }
}
