import { defineConfig, devices } from "@playwright/test";

// Phase-1 smoke: runs against a vite preview build. The test file injects a
// mock `@tauri-apps/api/core#invoke` so the UI exercises every screen
// without needing a running Tauri host — CI doesn't have one.
export default defineConfig({
    testDir: "./tests/e2e",
    timeout: 30_000,
    fullyParallel: false,
    retries: 0,
    reporter: "list",
    use: {
        baseURL: "http://localhost:4173",
        trace: "retain-on-failure",
    },
    projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
    webServer: {
        command: "pnpm build && pnpm preview --port 4173 --strictPort",
        url: "http://localhost:4173",
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
    },
});
