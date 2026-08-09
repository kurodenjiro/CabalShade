import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

/**
 * Two builds, selected by mode.
 *
 * `--mode mobile` builds the design-system UI; the default builds the frozen
 * desktop RPG UI. They are separate because Tailwind and framer-motion are
 * actively hostile to the design system — Tailwind compiles to the raw px and
 * hex values the adherence lint bans, and the brand forbids spring physics —
 * so the frozen tree keeps them and the mobile tree never sees them.
 *
 * Two output *directories*, not two entry files in one. `frontendDist` names a
 * directory and Tauri always serves the `index.html` inside it, so two entries
 * in one folder would load the desktop UI on the phone.
 */
export default defineConfig(({ mode }) => {
  const mobile = mode === "mobile";

  return {
    plugins: [react()],

    // The desktop bundle can be built from a read-only/managed node_modules
    // tree (common in CI and packaged Codex workspaces). Keep Vite's transient
    // config cache outside dependencies so the build does not depend on the
    // ownership of node_modules/.vite-temp.
    cacheDir: resolve("/tmp/cabalshade-vite-cache"),

    // Vite resolves `outDir` relative to `root`, so with a nested mobile root a
    // bare relative path would land inside src/mobile-entry/ and the Tauri
    // overlay would point at nothing. Absolute paths avoid that entirely.
    root: mobile ? resolve(rootDir, "src/mobile-entry") : rootDir,

    build: {
      // `dist-mobile` may be owned by a previous packaged build. Use a fresh
      // MVP output directory so a normal developer account can rebuild it.
      outDir: resolve(rootDir, mobile ? "dist-mobile-mvp" : "dist-desktop"),
      emptyOutDir: true,
    },

    // Prevent Vite from obscuring rust errors.
    clearScreen: false,

    server: {
      // @ts-expect-error process is a nodejs global
      port: parseInt(process.env.PORT || "1420"),
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            // A distinct port from the dev server. Sharing one meant HMR never
            // attached on a physical device.
            port: 1421,
          }
        : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
