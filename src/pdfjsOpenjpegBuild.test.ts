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
