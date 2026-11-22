import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { buildOpenjpeg } from "../scripts/build-pdfjs-openjpeg.mjs";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn() }));
vi.mock("../scripts/lib/pdfjs-openjpeg-abi.mjs", () => ({
  matchOpenjpegImports: (wasm: Uint8Array) => wasm,
}));

let outDir: string;
const compilerVersion = "emcc (Emscripten gcc/clang-like replacement) 5.0.6";

beforeEach(() => {
  fs.mkdirSync("target", { recursive: true });
  outDir = fs.mkdtempSync(path.resolve("target/pdfjs-compiler-test-"));
  vi.stubEnv("EMSDK", undefined);
  vi.stubEnv("TINE_PDFJS_EMSDK", undefined);
  vi.stubEnv("TINE_PDFJS_EMCC", undefined);
  vi.mocked(execFileSync).mockReturnValue(compilerVersion);
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  vi.mocked(execFileSync).mockReset();
  fs.rmSync(outDir, { recursive: true, force: true });
});

it("refuses a different compiler version in the default SDK layout", () => {
  vi.stubEnv("TINE_PDFJS_EMSDK", path.resolve("target/sdk/upstream"));
  expect(() => buildOpenjpeg({ outDir })).toThrow(/Expected Emscripten 4\.0\.20.*5\.0\.6/);
  expect(execFileSync).toHaveBeenCalledTimes(1);
});

it("refuses a packager compiler below the minimum before compiling", () => {
  vi.stubEnv("TINE_PDFJS_EMCC", "emcc");
  vi.mocked(execFileSync).mockReturnValue(compilerVersion.replace("5.0.6", "3.1.68"));
  expect(() => buildOpenjpeg({ outDir })).toThrow(/TINE_PDFJS_EMCC requires Emscripten 3\.1\.69 or newer.*3\.1\.68/);
  expect(execFileSync).toHaveBeenCalledTimes(1);
});

it.each(["emcc", path.resolve("target/packager/bin/emcc")])(
  "uses the packager's compiler and configuration with TINE_PDFJS_EMCC=%s",
  compiler => {
    const config = path.resolve("target/packager/emscripten.config");
    vi.stubEnv("TINE_PDFJS_EMCC", compiler);
    vi.stubEnv("EM_CONFIG", config);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.mocked(execFileSync).mockImplementationOnce(() => compilerVersion).mockImplementationOnce(() => {
      throw new Error("compiler reached");
    });

    expect(() => buildOpenjpeg({ outDir })).toThrow("compiler reached");
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("5.0.6"));
    expect(execFileSync).toHaveBeenNthCalledWith(1, compiler, ["--version"], expect.objectContaining({
      env: expect.objectContaining({ EM_CONFIG: config }),
    }));
    expect(execFileSync).toHaveBeenNthCalledWith(2, compiler, expect.arrayContaining(["-c"]), expect.objectContaining({
      env: expect.objectContaining({ EM_CONFIG: config }),
    }));
    expect(fs.existsSync(path.join(outDir, "emscripten.config"))).toBe(false);
  },
);

it("runs a packager's tools as spelled from the caller's directory and environment (review r2 F2)", () => {
  vi.stubEnv("TINE_PDFJS_EMCC", "tools/bin/emcc");
  vi.stubEnv("EM_CONFIG", "~/emscripten.config");
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.mocked(execFileSync).mockImplementationOnce(() => compilerVersion).mockImplementationOnce(() => {
    throw new Error("compiler reached");
  });
  expect(() => buildOpenjpeg({ outDir })).toThrow("compiler reached");
  const [, args, options] = vi.mocked(execFileSync).mock.calls[1];
  expect(vi.mocked(execFileSync).mock.calls[1][0]).toBe("tools/bin/emcc");
  expect(options).toMatchObject({ cwd: undefined, env: expect.objectContaining({ EM_CONFIG: "~/emscripten.config" }) });
  expect(args).toContain(path.join(outDir, "obj/thread.o"));
});

it.each([["emcc", "emar"], ["./emcc", "./emar"], ["link/../bin/emcc", "link/../bin/emar"]])(
  "pairs TINE_PDFJS_EMCC=%s with the archiver %s, keeping its spelling (review r3 F2)", (emcc, emar) => {
    vi.stubEnv("TINE_PDFJS_EMCC", emcc);
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.mocked(execFileSync).mockImplementation(((file: string) => {
      if (file.endsWith("emar")) throw new Error(`archiver ${file}`);
      return compilerVersion;
    }) as typeof execFileSync);
    expect(() => buildOpenjpeg({ outDir })).toThrow(`archiver ${emar}`);
  });

describe("decoder cache", () => {
  const source = "openjpeg/openjp2/thread.c";
  const vendor = "vendor/pdfjs-openjpeg";
  const tools = vi.mocked(execFileSync);

  function buildAndCountTools() {
    tools.mockClear();
    expect(buildOpenjpeg({ outDir })).toBe(outDir);
    const calls = tools.mock.calls.map(([file, args = []]) =>
      file === "test-python" ? [path.basename(args[0], ".py"), ...args.slice(1)] : [file, ...args]);
    expect(calls).toHaveLength(25);
    expect(calls.filter(([tool, flag]) => tool === "emcc" && flag === "--version")).toHaveLength(1);
    expect(calls.filter(([tool, flag]) => tool === "emcc" && flag === "-c")).toHaveLength(22);
    expect(calls.filter(([tool, flag]) => tool === "emar" && flag === "rcs")).toHaveLength(1);
    expect(calls.filter(([tool, flag]) => tool === "emcc" && flag === "-o")).toHaveLength(1);
  }

  beforeEach(() => {
    vi.stubEnv("PYTHON", "test-python");
    vi.stubEnv("TINE_PDFJS_EMSDK", path.resolve("target/sdk/upstream"));
    tools.mockImplementation(((file: string, args: string[]) => {
      const flag = args[file === "test-python" ? 1 : 0];
      if (flag === "--version") {
        return file === "test-python" ? compilerVersion.replace("5.0.6", "4.0.20") : compilerVersion;
      }
      if (flag === "-o") {
        fs.writeFileSync(path.join(outDir, "openjpeg.js"), "");
        fs.writeFileSync(path.join(outDir, "openjpeg.wasm"), "test decoder bytes");
      }
      return "";
    }) as typeof execFileSync);
    buildAndCountTools();
  });

  it("reuses the pinned decoder without launching a tool or needing the SDK", () => {
    vi.stubEnv("TINE_PDFJS_EMSDK", undefined);
    tools.mockClear();
    expect(buildOpenjpeg({ outDir })).toBe(outDir);
    expect(tools).toHaveBeenCalledTimes(0);
  });

  it.each([
    `${vendor}/${source}`,
    "node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs",
    "scripts/build-pdfjs-openjpeg.mjs",
    "scripts/lib/pdfjs-openjpeg-abi.mjs",
  ])("rebuilds the pinned decoder when %s changes", input => {
    const read = fs.readFileSync;
    const changedFile = path.resolve(input);
    const changedBytes = Buffer.concat([read(changedFile), Buffer.from("\n")]);
    const provenanceFile = path.resolve(vendor, "source.json");
    vi.spyOn(fs, "readFileSync").mockImplementation(((...args: Parameters<typeof read>) => {
      const [file] = args;
      if (file === changedFile) return changedBytes;
      if (input === `${vendor}/${source}` && file === provenanceFile) {
        // A revised vendored source comes with its matching provenance hash.
        const provenance = JSON.parse(read(provenanceFile, "utf8"));
        provenance.files[source] = createHash("sha256").update(changedBytes).digest("hex");
        return JSON.stringify(provenance);
      }
      return read(...args);
    }) as typeof read);
    buildAndCountTools();
  });

  it("rebuilds when only the cached WASM bytes change", () => {
    fs.appendFileSync(path.join(outDir, "openjpeg.wasm"), "corruption");
    buildAndCountTools();
    expect(fs.readFileSync(path.join(outDir, "openjpeg.wasm"), "utf8")).toBe("test decoder bytes");
  });

  it("rebuilds on every packager invocation and keeps its receipt separate from the pinned build", () => {
    vi.stubEnv("TINE_PDFJS_EMCC", "emcc");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    buildAndCountTools(); // The output directory holds a valid pinned receipt.
    buildAndCountTools(); // Same packager, with its own valid receipt on disk.
    vi.stubEnv("TINE_PDFJS_EMCC", undefined);
    buildAndCountTools(); // A pinned build must also miss after a packager build.
  });

  it("builds with Emscripten 3.1.69 using the worker's runtime ABI", () => {
    vi.stubEnv("TINE_PDFJS_EMCC", "emcc");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    tools.mockReturnValueOnce(compilerVersion.replace("5.0.6", "3.1.69"));
    buildAndCountTools();
    expect(tools).toHaveBeenLastCalledWith("emcc", expect.arrayContaining([
      "-mbulk-memory", "-sWASM_BIGINT=1", "-sMINIMAL_RUNTIME=1", "-Wl,--wrap=__wasi_fd_close",
    ]), expect.anything());
  });
});
