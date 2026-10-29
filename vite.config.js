import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import process from "node:process";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
const host = process.env.TAURI_DEV_HOST;

// Excalidraw loads its fonts from window.EXCALIDRAW_ASSET_PATH (src/lib/excalidraw.ts)
// and then from a CDN, which the CSP blocks. The app serves them itself.
// Xiaolai (13 MB of CJK glyphs) is left out: that text uses system fonts instead.
const EXCALIDRAW_CDN = /("ASSETS_FALLBACK_URL",\s*)`https:\/\/esm\.sh\/.*?\/dist\/prod\/`/;
/** Points Excalidraw's CDN fallback at the app's own fonts, also while dependencies are pre-bundled. */
function excalidrawLocalFallback() {
  return {
    name: "lownotes-excalidraw-local-fallback",
    /** @param {string} code @param {string} id */
    transform(code, id) {
      if (!id.includes("@excalidraw") || !code.includes('"ASSETS_FALLBACK_URL"')) return null;
      if (!EXCALIDRAW_CDN.test(code)) throw new Error(`Excalidraw's font fallback changed in ${id}; update vite.config.js`);
      return { code: code.replace(EXCALIDRAW_CDN, '$1new URL("/excalidraw/", window.location.href).href'), map: null };
    },
  };
}
const excalidrawFonts = fileURLToPath(new URL("./node_modules/@excalidraw/excalidraw/dist/prod/fonts/", import.meta.url));
function excalidrawFontFiles() {
  return readdirSync(excalidrawFonts, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".woff2"))
    .map((entry) => relative(excalidrawFonts, join(entry.parentPath, entry.name)).split(sep).join("/"))
    .filter((path) => !path.startsWith("Xiaolai/"));
}
/** @returns {import("vite").Plugin} */
function excalidrawAssets() {
  return {
    name: "lownotes-excalidraw-assets",
    configureServer(server) {
      const files = new Set(excalidrawFontFiles());
      server.middlewares.use("/excalidraw/fonts/", (request, response, next) => {
        const path = decodeURIComponent((request.url ?? "").split("?")[0].replace(/^\/+/, ""));
        if (!files.has(path)) return next();
        response.setHeader("Content-Type", "font/woff2");
        response.end(readFileSync(join(excalidrawFonts, path)));
      });
    },
    generateBundle() {
      if (this.environment?.config.consumer !== "client") return;
      for (const path of excalidrawFontFiles()) {
        this.emitFile({ type: "asset", fileName: `excalidraw/fonts/${path}`, source: readFileSync(join(excalidrawFonts, path)) });
      }
    },
  };
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [tailwindcss(), sveltekit(), excalidrawAssets(), excalidrawLocalFallback()],
  optimizeDeps: { rolldownOptions: { plugins: [excalidrawLocalFallback()] } },
  // Only the version is needed at runtime; importing package.json would ship
  // the whole dependency list in the bundle.
  define: {
    __APP_VERSION__: JSON.stringify(JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")).version),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
