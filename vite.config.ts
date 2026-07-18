import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";

// Build timestamp, shown in Settings so it's easy to confirm the running binary is
// the latest (vs. a stale Syncthing copy). Wall-clock by default, so "Built" tracks
// the ACTUAL build moment — a re-deploy of the same commit, or a stale synced copy,
// can no longer masquerade as fresh. Reproducible builds (F-Droid) set
// SOURCE_DATE_EPOCH to the commit unix time; when present we pin to it so the bundle
// stays byte-identical across machines. (We used to fall back to the *commit* time
// even for local builds, which froze "Built" between commits even as you rebuilt.)
function reproBuildTime(): string {
  const epoch = process.env.SOURCE_DATE_EPOCH;
  return epoch ? new Date(Number(epoch) * 1000).toISOString() : new Date().toISOString();
}
const BUILD_TIME = reproBuildTime();

// Short commit the bundle was built from — shown in About so an issue reporter
// can name the exact build. Empty string if git isn't available (e.g. a source
// tarball build); the About tab hides the row when it's empty.
function gitCommit(): string {
  try {
    return execSync("git rev-parse --short HEAD", { encoding: "utf8" }).trim();
  } catch {
    return "";
  }
}
const GIT_COMMIT = gitCommit();

// The @twemoji/svg package holds one <codepoint>.svg per emoji at its root.
const twemojiDir = fileURLToPath(new URL("./node_modules/@twemoji/svg", import.meta.url));

// Emoji are rendered as Twemoji SVG <img>s (render/emoji.tsx), NOT a color-emoji
// font (WebKitGTK paints those blank). This plugin serves the SVGs at
// `/twemoji/<codepoint>.svg` in dev/preview and copies them into `dist/twemoji/`
// at build, so the bundled Tauri app (and `vite preview`) can load them offline.
function twemojiAssets(): Plugin {
  const serve = (req: IncomingMessage, res: ServerResponse, next: () => void) => {
    const url = req.url || "";
    if (url.startsWith("/twemoji/")) {
      const name = path.basename(url.split("?")[0]);
      const file = path.join(twemojiDir, name);
      if (name.endsWith(".svg") && fs.existsSync(file)) {
        res.setHeader("Content-Type", "image/svg+xml");
        res.setHeader("Cache-Control", "max-age=31536000, immutable");
        fs.createReadStream(file).pipe(res);
        return;
      }
    }
    next();
  };
  return {
    name: "twemoji-assets",
    configureServer(server) {
      server.middlewares.use(serve);
    },
    configurePreviewServer(server) {
      server.middlewares.use(serve);
    },
    async writeBundle() {
      const dest = fileURLToPath(new URL("./dist/twemoji", import.meta.url));
      await fsp.mkdir(dest, { recursive: true });
      const files = (await fsp.readdir(twemojiDir)).filter((f) => f.endsWith(".svg"));
      await Promise.all(
        files.map((f) => fsp.copyFile(path.join(twemojiDir, f), path.join(dest, f)))
      );
    },
  };
}

// Tauri expects a fixed port and serves the built assets from dist/.
export default defineConfig({
  plugins: [solid(), twemojiAssets()],
  define: {
    __BUILD_TIME__: JSON.stringify(BUILD_TIME),
    __GIT_COMMIT__: JSON.stringify(GIT_COMMIT),
    // The network-backed community plugin/theme registry. Disabled when the
    // build sets TINE_COMMUNITY_REGISTRY=0 — the F-Droid recipe does this so the
    // published build never fetches executable plugin code at runtime (F-Droid
    // inclusion policy). Local sideloading is unaffected. Default: enabled.
    __TINE_COMMUNITY_REGISTRY__: JSON.stringify(process.env.TINE_COMMUNITY_REGISTRY !== "0"),
  },
  clearScreen: false,
  server: {
    port: 5181,
    strictPort: true,
  },
  build: {
    target: "esnext",
    outDir: "dist",
    rollupOptions: {
      // Two HTML entries: the main app and the standalone quick-capture window.
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        capture: fileURLToPath(new URL("./capture.html", import.meta.url)),
      },
    },
  },
});
