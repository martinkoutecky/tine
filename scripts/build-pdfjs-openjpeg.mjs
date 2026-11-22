// Build the JPX decoder from vendored upstream C sources. No network access,
// CMake, native target compiler, or prebuilt decoder is required.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { matchOpenjpegImports } from "./lib/pdfjs-openjpeg-abi.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
export const openjpegBuildDir = path.join(root, "target/pdfjs-openjpeg");
const vendor = path.join(root, "vendor/pdfjs-openjpeg");
const noFs = fileURLToPath(new URL("./lib/pdfjs-openjpeg-nofs.c", import.meta.url));
const minimumPackagerVersion = "3.1.69";
const packagerHelp = `Set TINE_PDFJS_EMCC to use a packager toolchain (Emscripten ${minimumPackagerVersion} or newer).`;
const sha = bytes => crypto.createHash("sha256").update(bytes).digest("hex");
const read = file => fs.readFileSync(file, "utf8");

export function buildOpenjpeg({ outDir = openjpegBuildDir, force = false } = {}) {
  const emcc = process.env.TINE_PDFJS_EMCC;
  // A packager's tools run as spelled, from the caller's directory, with the caller's environment.
  // The sibling archiver keeps the compiler's spelling (./, .., symlinks); a bare emcc pairs with a bare emar.
  const compiler = emcc && { emcc, emar: emcc.slice(0, emcc.length - path.basename(emcc).length) + "emar" };
  const sdk = process.env.TINE_PDFJS_EMSDK || (process.env.EMSDK && path.join(process.env.EMSDK, "upstream"));
  const config = path.join(outDir, "emscripten.config");
  const env = compiler ? process.env
    : { ...process.env, EM_CONFIG: config, EM_CACHE: path.join(outDir, "cache"), EMCC_CORES: "4" };
  // The pinned build runs in outDir with relative names (its bytes stay reproducible); a packager's names are absolute.
  const at = name => compiler ? path.join(outDir, name) : name;
  const python = process.env.PYTHON || (process.platform === "win32" ? "python" : "python3");
  const tool = (name, args, capture = false) => execFileSync(compiler ? compiler[name] : python,
    compiler ? args : [path.join(sdk, "emscripten", `${name}.py`), ...args],
    { cwd: compiler ? undefined : outDir, env, encoding: "utf8", stdio: capture ? "pipe" : "inherit" });

  const provenance = JSON.parse(read(path.join(vendor, "source.json")));
  const hash = crypto.createHash("sha256");
  for (const [file, expected] of Object.entries(provenance.files)) {
    const bytes = fs.readFileSync(path.join(vendor, file));
    if (sha(bytes) !== expected) throw new Error(`Vendored OpenJPEG source changed: ${file}`);
    hash.update(file).update(bytes);
  }
  const worker = fs.readFileSync(path.join(root, "node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs"));
  hash.update(worker);
  for (const file of [fileURLToPath(import.meta.url), fileURLToPath(new URL("./lib/pdfjs-openjpeg-abi.mjs", import.meta.url))]) {
    hash.update(fs.readFileSync(file));
  }
  // A packager's toolchain cannot be fingerprinted, so its decoder is never reused, and its receipt never
  // carries the pinned key (a later default build must not ship it).
  if (compiler) hash.update("packager").update(fs.readFileSync(noFs));
  const inputHash = hash.digest("hex"), output = path.join(outDir, "openjpeg.wasm");
  const receipt = path.join(outDir, "BUILD_OPENJPEG.json");
  if (!force && !compiler && fs.existsSync(receipt) && fs.existsSync(output)) {
    const previous = JSON.parse(read(receipt));
    if (previous.inputHash === inputHash && previous.sha256 === sha(fs.readFileSync(output))) return outDir;
  }
  if (!compiler && !sdk) throw new Error(`Building the PDF decoder requires Emscripten 4.0.20 via EMSDK or TINE_PDFJS_EMSDK. ${packagerHelp} See 'PDF decoder SDK' in README.md.`);
  fs.mkdirSync(path.join(outDir, "obj"), { recursive: true });
  if (!compiler) fs.writeFileSync(config, `LLVM_ROOT = ${JSON.stringify(path.join(sdk, "bin"))}\nBINARYEN_ROOT = ${JSON.stringify(sdk)}\nNODE_JS = [${JSON.stringify(process.execPath)}]\n`);
  const version = tool("emcc", ["--version"], true).split("\n")[0];
  const release = /\b\d+\.\d+\.\d+\b/.exec(version)?.[0];
  const [major, minor, patch] = release?.split(".").map(Number) ?? [];
  const [minMajor, minMinor, minPatch] = minimumPackagerVersion.split(".").map(Number);
  if (compiler && (!release || major < minMajor || (major === minMajor &&
    (minor < minMinor || (minor === minMinor && patch < minPatch))))) {
    throw new Error(`TINE_PDFJS_EMCC requires Emscripten ${minimumPackagerVersion} or newer, got ${version}.`);
  }
  if (release !== "4.0.20") {
    if (!compiler) throw new Error(`Expected Emscripten 4.0.20, got ${version}. ${packagerHelp}`);
    console.warn(`Building the PDF decoder with TINE_PDFJS_EMCC and ${version} (pinned release version: 4.0.20).`);
  }
  const legacyExports = Boolean(compiler && major === 3);
  // Bulk memory replaces the JS memcpy import. BigInt matches the worker's i64 fd_seek.
  const legacyLink = legacyExports ? ["-mbulk-memory", "-sWASM_BIGINT=1", "-sMINIMAL_RUNTIME=1",
    noFs, "-Wl,--wrap=__wasi_fd_close"] : [];
  const runtimeMethods = legacyExports ? ["writeArrayToMemory", "wasmTable"] : ["writeArrayToMemory"];
  // Fixed wasm32 target configuration, equivalent to upstream's CMake probes.
  fs.writeFileSync(path.join(outDir, "opj_config.h"), "#define OPJ_VERSION_MAJOR 2\n#define OPJ_VERSION_MINOR 5\n#define OPJ_VERSION_BUILD 4\n");
  fs.writeFileSync(path.join(outDir, "opj_config_private.h"), '#define OPJ_PACKAGE_VERSION "2.5.4"\n#define OPJ_HAVE_FSEEKO 1\n#define OPJ_HAVE_MALLOC_H\n#define OPJ_HAVE_MEMALIGN\n#define OPJ_HAVE_POSIX_MEMALIGN\n#ifndef _POSIX_C_SOURCE\n#define _POSIX_C_SOURCE 200112L\n#endif\n');
  fs.writeFileSync(path.join(outDir, "opj_apps_config.h"), '#include "opj_config_private.h"\n');
  const lib = path.join(vendor, "openjpeg/openjp2");
  const kernels = ["thread", "bio", "cio", "dwt", "event", "ht_dec", "image", "invert", "j2k", "jp2", "mct", "mqc",
    "openjpeg", "opj_clock", "pi", "t1", "t2", "tcd", "tgt", "function_list", "opj_malloc", "sparse_array"];
  const flags = ["-O3", "-msimd128", "-msse", "-DNDEBUG"];
  for (const name of kernels) {
    tool("emcc", ["-c", path.join(lib, `${name}.c`), "-o", at(`obj/${name}.o`), `-I${lib}`, `-I${outDir}`, "-DMUTEX_stub", ...flags]);
  }
  tool("emar", ["rcs", at("libopenjp2.a"), ...kernels.map(name => at(`obj/${name}.o`))]);
  tool("emcc", ["-o", at("openjpeg.js"), at("libopenjp2.a"), path.join(vendor, "openjpeg/convert.c"),
    path.join(vendor, "openjpeg/color.c"), path.join(vendor, "bridge/decoder.c"),
    `-I${lib}`, `-I${path.join(vendor, "openjpeg")}`, `-I${outDir}`,
    "-sALLOW_MEMORY_GROWTH=1", "-sMAXIMUM_MEMORY=2GB", "-sWASM=1", "-sMODULARIZE=1",
    "-sEXPORT_NAME=OpenJPEG", "-sWASM_ASYNC_COMPILATION=1", "-sEXPORT_ES6=1", "-sSINGLE_FILE=0",
    "-sENVIRONMENT=web", "-sERROR_ON_UNDEFINED_SYMBOLS=1", "-sNO_FILESYSTEM=1", "-sNO_EXIT_RUNTIME=1",
    "-sMALLOC=emmalloc", '-sEXPORTED_FUNCTIONS=["_jp2_decode","_malloc","_free"]',
    `-sEXPORTED_RUNTIME_METHODS=${JSON.stringify(runtimeMethods)}`, "-sASSERTIONS=0", "-flto", ...flags,
    "--js-library", path.join(vendor, "bridge/myjs.js"), ...legacyLink]);
  const wasm = matchOpenjpegImports(fs.readFileSync(output), read(path.join(outDir, "openjpeg.js")), worker.toString(), { legacyExports });
  fs.writeFileSync(output, wasm);
  fs.writeFileSync(path.join(outDir, "LICENSE_OPENJPEG"), read(path.join(vendor, "LICENSE_OPENJPEG")) + "\n" + read(path.join(vendor, "LICENSE_BRIDGE")));
  fs.writeFileSync(receipt, JSON.stringify({ source: provenance.upstream, compiler: version, inputHash,
    simd: "simd128", relaxedSimd: false, initialMemoryBytes: 16777216, maximumMemoryBytes: 2147483648,
    sha256: sha(wasm) }, null, 2) + "\n");
  return outDir;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  // A packager's decoder is never reused, so within `npm run build` only the Vite asset step compiles it.
  if (process.env.TINE_PDFJS_EMCC && process.argv.includes("--full-build")) console.log("OpenJPEG: TINE_PDFJS_EMCC is set; the Vite asset step builds the decoder.");
  else console.log(`OpenJPEG standard SIMD: ${buildOpenjpeg()}`);
}
