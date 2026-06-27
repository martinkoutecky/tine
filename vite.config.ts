import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";

// Build timestamp, stamped at bundle time and shown in Settings so it's easy to
// confirm the running binary is the latest (vs. a stale Syncthing copy).
const BUILD_TIME = new Date().toISOString();

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
