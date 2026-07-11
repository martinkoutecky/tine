import { MAX_EVENT_BYTES, MAX_RESPONSE_BYTES, parsePluginResponse, type PluginEvent, type PluginResponse } from "./protocol";

const WASM_PAGE_BYTES = 64 * 1024;
const REQUIRED_EXPORTS = ["tine_alloc", "tine_handle", "tine_result_len"] as const;

export interface PluginGuestLimits {
  memoryInitialPages: number;
  memoryMaximumPages: number;
}

export const DEFAULT_GUEST_LIMITS: PluginGuestLimits = {
  memoryInitialPages: 4,
  memoryMaximumPages: 256,
};

export class PluginGuestError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginGuestError";
  }
}

interface GuestExports extends WebAssembly.Exports {
  tine_alloc: (length: number) => number;
  tine_handle: (pointer: number, length: number) => number;
  tine_result_len: () => number;
}

export interface PluginGuest {
  memory: WebAssembly.Memory;
  exports: GuestExports;
}

function safeInteger(value: number, label: string): number {
  const normalized = value >>> 0;
  if (!Number.isSafeInteger(normalized)) throw new PluginGuestError(`${label} is invalid`);
  return normalized;
}

function assertRegion(memory: WebAssembly.Memory, pointer: number, length: number, label: string) {
  if (length < 0 || pointer > memory.buffer.byteLength || length > memory.buffer.byteLength - pointer) {
    throw new PluginGuestError(`${label} lies outside guest memory`);
  }
}

export async function instantiatePluginGuest(
  wasm: ArrayBuffer,
  limits: PluginGuestLimits = DEFAULT_GUEST_LIMITS
): Promise<PluginGuest> {
  if (
    !Number.isInteger(limits.memoryInitialPages) ||
    !Number.isInteger(limits.memoryMaximumPages) ||
    limits.memoryInitialPages < 1 ||
    limits.memoryMaximumPages < limits.memoryInitialPages ||
    limits.memoryMaximumPages > 1_024
  ) {
    throw new PluginGuestError("invalid guest memory limits");
  }
  let module: WebAssembly.Module;
  try {
    module = await WebAssembly.compile(wasm);
  } catch {
    throw new PluginGuestError("entry is not a valid WebAssembly module");
  }
  const imports = WebAssembly.Module.imports(module);
  if (
    imports.length !== 1 ||
    imports[0].module !== "env" ||
    imports[0].name !== "memory" ||
    imports[0].kind !== "memory"
  ) {
    throw new PluginGuestError("guest must import exactly env.memory and no ambient APIs");
  }
  const exported = new Map(WebAssembly.Module.exports(module).map((entry) => [entry.name, entry.kind]));
  for (const name of REQUIRED_EXPORTS) {
    if (exported.get(name) !== "function") throw new PluginGuestError(`guest is missing function export ${name}`);
  }
  const memory = new WebAssembly.Memory({
    initial: limits.memoryInitialPages,
    maximum: limits.memoryMaximumPages,
  });
  let instance: WebAssembly.Instance;
  try {
    instance = await WebAssembly.instantiate(module, { env: { memory } });
  } catch (error) {
    const detail = error instanceof Error ? `: ${error.message}` : "";
    throw new PluginGuestError(`guest memory requirements exceed the host limit${detail}`);
  }
  const exports = instance.exports as GuestExports;
  if (
    typeof exports.tine_alloc !== "function" ||
    typeof exports.tine_handle !== "function" ||
    typeof exports.tine_result_len !== "function"
  ) {
    throw new PluginGuestError("guest ABI exports have invalid types");
  }
  return { memory, exports };
}

export function invokePluginGuest(guest: PluginGuest, event: PluginEvent): PluginResponse {
  const encoder = new TextEncoder();
  const bytes = encoder.encode(JSON.stringify(event));
  if (bytes.byteLength > MAX_EVENT_BYTES) throw new PluginGuestError("event exceeds the guest input limit");

  const inputPointer = safeInteger(guest.exports.tine_alloc(bytes.byteLength), "input pointer");
  assertRegion(guest.memory, inputPointer, bytes.byteLength, "input");
  new Uint8Array(guest.memory.buffer, inputPointer, bytes.byteLength).set(bytes);

  const outputPointer = safeInteger(guest.exports.tine_handle(inputPointer, bytes.byteLength), "output pointer");
  const outputLength = safeInteger(guest.exports.tine_result_len(), "output length");
  if (outputLength > MAX_RESPONSE_BYTES) throw new PluginGuestError("response exceeds the guest output limit");
  assertRegion(guest.memory, outputPointer, outputLength, "output");

  let decoded: unknown;
  try {
    const output = new Uint8Array(guest.memory.buffer, outputPointer, outputLength);
    decoded = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(output));
  } catch {
    throw new PluginGuestError("guest returned invalid UTF-8 JSON");
  }
  return parsePluginResponse(decoded);
}

export function guestMaximumBytes(limits: PluginGuestLimits): number {
  return limits.memoryMaximumPages * WASM_PAGE_BYTES;
}
