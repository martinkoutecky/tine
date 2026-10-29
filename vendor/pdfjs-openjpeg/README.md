# OpenJPEG for pdf.js 5.7.284

Prior decisions: D-20261028-PDF9 item 7 and D-20261029-PJ5 (follow): upgrade
pdf.js without regressing scanned-PDF performance or memory. Keep Tine's
existing bounded page/tile caches and guarded graph writers (follow).

`source.json` pins OpenJPEG 2.5.4 and Mozilla's decoder bridge by upstream
commit, and hashes every vendored source and licence. `openjpeg/openjp2/`
comes from OpenJPEG's `src/lib/openjp2/`; `convert.{c,h}` comes from
`src/bin/jp2/`, and `color.{c,h}` and `opj_getopt.h` from `src/bin/common/`.
The bridge's upstream buffer-stream patch is already applied to the OpenJPEG
sources; the original patch is included. No decoder algorithm is changed.
Both upstreams use BSD-2-Clause; the build bundles both licence texts.

`scripts/build-pdfjs-openjpeg.mjs` reproduces the upstream wasm32/no-threads
configuration and compiles with Emscripten 4.0.20, `-O3 -msimd128 -msse`.
It omits upstream's `-mrelaxed-simd`, which causes WebKitGTK to reject the
official module and use its much slower JS fallback. The initial heap stays
16 MiB, growth stays enabled, and the maximum stays 2 GiB. The compiler, source
hashes, memory settings and output digest are recorded in `BUILD_OPENJPEG.json`.

The compiler may minify import names differently. The build reads the semantic
import names from the generated glue and pdf.js's bundled OpenJPEG glue, then
renames only the WASM import strings to match. It rejects changed exports,
missing symbols or an unexpected WASM interface. It leaves the worker,
decoder instructions, types, memory limits and official JS fallback unchanged.
The Vite plugin serves the same rebuilt WASM in development and copies it
with its licences and provenance into the offline application bundle.

Install/activate Emscripten 4.0.20 and source `emsdk_env.sh`, then run
`npm run build:pdf-decoder`. `EMSDK` identifies the SDK installation;
`TINE_PDFJS_EMSDK` may instead name its `upstream/` directory. Python 3 and
Node.js 22.13+ must be available. The builder needs no CMake, Docker, network,
or host C compiler. All intermediate output and the compiler cache stay in
`artifacts/pdfjs5/decoder/`. A verified source/output hash enables reuse.

CI installs the pinned SDK before building. Flatpak materializes the SDK
archive as a pinned manifest source for each Linux host architecture, then
sets `TINE_PDFJS_EMSDK`; compilation uses the SDK's local sysroot sources and
the Flatpak Node executable. `npm run build` performs no download.

To check a fresh reconstruction without reusing the output cache:

```sh
node --input-type=module -e 'import {buildOpenjpeg} from "./scripts/build-pdfjs-openjpeg.mjs"; buildOpenjpeg({outDir:process.cwd()+"/artifacts/pdfjs5/repro",force:true})'
```

Compare its `openjpeg.wasm` SHA-256 with the normal build. Native acceptance
also checks that WebKitGTK validates this module and that real scanned pages
retain their pixels; Node's WASM support alone cannot establish compatibility.
