import { resolve } from "node:path";
import { defineConfig } from "vite";

const root = resolve(__dirname, "src");
const pages = ["overlay", "frame", "toolbar", "stack", "settings"];

export default defineConfig({
  root,
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: resolve(__dirname, "dist"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: Object.fromEntries(
        pages.map((p) => [p, resolve(root, `windows/${p}/index.html`)]),
      ),
    },
  },
});
