import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { buildOpenjpeg } from "../scripts/build-pdfjs-openjpeg.mjs";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn() }));

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

it("rebuilds when the cached decoder was made under another key (review F1; both directions are proved with real toolchains in the r3 evidence)", () => {
  vi.stubEnv("TINE_PDFJS_EMCC", "emcc");
  vi.spyOn(console, "warn").mockImplementation(() => {});
  fs.writeFileSync(path.join(outDir, "openjpeg.wasm"), "cached");
  fs.writeFileSync(path.join(outDir, "BUILD_OPENJPEG.json"), JSON.stringify({ inputHash: "x", sha256: "y" }));
  vi.mocked(execFileSync).mockImplementationOnce(() => compilerVersion).mockImplementationOnce(() => {
    throw new Error("compiler reached");
  });
  expect(() => buildOpenjpeg({ outDir })).toThrow("compiler reached");
});
