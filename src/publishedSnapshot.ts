/** Generation-2 source document shared by folder publications and quick shares. */
export interface PublishedSnapshot {
  schema: 2;
  name: string;
  exported_at: string;
  files: { path: string; text: string }[];
}

/** Read at most `max` bytes, checking the declared length and cancelling an oversize stream. */
export async function readBounded(response: Response, max: number, label: string): Promise<Uint8Array<ArrayBuffer>> {
  const tooLarge = () => new Error(`${label}: larger than ${max} bytes`);
  const declared = Number(response.headers?.get("content-length") ?? "");
  if (Number.isFinite(declared) && declared > max) throw tooLarge();
  const reader = response.body?.getReader();
  if (!reader) {
    const whole = new Uint8Array(await response.arrayBuffer());
    if (whole.byteLength > max) throw tooLarge();
    return whole;
  }
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > max) {
      // The oversize refusal is the answer; a failed cancel is attached, not dropped.
      try {
        await reader.cancel();
      } catch (cancelError) {
        throw new Error(tooLarge().message, { cause: cancelError });
      }
      throw tooLarge();
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.byteLength;
  }
  return out;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Admit the flat source snapshot. The core checks graph paths and reads the files. */
export function validateSnapshot(value: unknown): PublishedSnapshot {
  const schema = 2;
  if (!record(value) || value.schema !== schema) throw new Error(`Expected snapshot schema ${schema}`);
  if (typeof value.name !== "string" || typeof value.exported_at !== "string" || !Array.isArray(value.files)
    || !value.files.every((file) => record(file) && typeof file.path === "string" && typeof file.text === "string")) {
    throw new Error("Invalid publication snapshot");
  }
  return { schema, name: value.name, exported_at: value.exported_at, files: value.files };
}
