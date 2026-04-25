import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
    plugins: [react()],

    // Prevent Vite from obscuring Rust errors
    clearScreen: false,
    server: {
        port: 5180,
        strictPort: true,
        host: host || false,
        hmr: host
            ? {
                  protocol: "ws",
                  host,
                  port: 5174,
              }
            : undefined,
        watch: {
            ignored: ["**/src-tauri/**"],
        },
    },
    test: {
        environment: "jsdom",
        globals: true,
        setupFiles: ["./vitest.setup.ts"],
        // Playwright e2e tests live under tests/e2e and are driven by the
        // Playwright runner, not vitest.
        exclude: ["node_modules", "dist", "tests/e2e/**"],
    },
}));
