import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
    plugins: [react()],

    // Prevent Vite from obscuring Rust errors
    clearScreen: false,
    server: {
        port: 5173,
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
    },
}));
