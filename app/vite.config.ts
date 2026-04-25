import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
    plugins: [react()],

    // Prevent Vite from obscuring Rust errors
    clearScreen: false,

    // Phase 9 build polish: split vendor + framer-motion + radix into
    // their own chunks so the warning about >500 KB main bundle goes
    // away. Each chunk lazy-loads naturally because Tauri bundles
    // everything into the same WebView once the user is past Unlock —
    // the split only matters at first paint.
    build: {
        rollupOptions: {
            output: {
                manualChunks: {
                    react: ["react", "react-dom", "react-dom/client"],
                    query: ["@tanstack/react-query", "@tanstack/react-virtual"],
                    motion: ["framer-motion"],
                    radix: [
                        "@radix-ui/react-dialog",
                        "@radix-ui/react-dropdown-menu",
                        "@radix-ui/react-popover",
                        "@radix-ui/react-tooltip",
                        "@radix-ui/react-tabs",
                        "@radix-ui/react-toggle-group",
                        "@radix-ui/react-scroll-area",
                        "@radix-ui/react-slider",
                        "@radix-ui/react-toast",
                    ],
                    cmdk: ["cmdk"],
                    icons: ["lucide-react"],
                },
            },
        },
    },

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
