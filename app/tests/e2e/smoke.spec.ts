import { test, expect } from "@playwright/test";

// End-to-end smoke driven against a vite preview build. The Tauri `invoke`
// call is stubbed via `window.__MV_MOCK_IPC__` so every command round-trips
// through a deterministic in-memory backend.
//
// This exercises:
//   - first-run create-user flow
//   - source add + progress
//   - timeline paging (1 mock asset)
//   - asset detail
//   - album create + detail + export stub

test("create user → add source → timeline → album export", async ({ page }) => {
    await page.addInitScript(() => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const state: any = {
            userExists: false,
            session: null,
            sources: [] as unknown[],
            albums: [] as unknown[],
            assets: [] as unknown[],
        };

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (window as any).__MV_MOCK_IPC__ = async (cmd: string, args: any) => {
            switch (cmd) {
                case "user_exists":
                    return state.userExists;
                case "list_users":
                    // D6 multi-user — Unlock prefers list_users over
                    // user_exists so it can pre-populate the username
                    // dropdown. Return shape: UserSummaryView[].
                    return state.userExists
                        ? [
                              {
                                  user_id: 1,
                                  username: state.session?.username ?? "alice",
                                  created_at: 0,
                              },
                          ]
                        : [];
                case "list_local_peers":
                    return [];
                case "create_user":
                    state.userExists = true;
                    state.session = {
                        user_id: 1,
                        username: args.username,
                        default_collection_id: 1,
                        hidden_unlocked: false,
                    };
                    return state.session;
                case "list_sources":
                    return state.sources;
                case "add_source":
                    state.sources.push({
                        id: state.sources.length + 1,
                        name: args.name,
                        root_path: args.root,
                        adapter_kind: args.adapter,
                        linked_only: args.linkedOnly,
                        bytes_total: 2048,
                        file_count: 1,
                        imported_at: 0,
                    });
                    state.assets.push({ id: 1, mime: "image/jpeg", is_video: false, is_live: false });
                    return state.sources.length;
                case "ingest_status":
                    return {
                        source_id: args.sourceId,
                        state: { state: "done", inserted: 1, deduped: 0, skipped: 0, errors: 0 },
                    };
                case "timeline_page":
                    return { entries: state.assets, next_cursor: null };
                case "asset_thumbnail":
                case "asset_original":
                    return [];
                case "asset_detail":
                    return {
                        id: args.id,
                        mime: "image/x-canon-cr2",
                        bytes: 2048,
                        width: 1920,
                        height: 1080,
                        duration_ms: null,
                        taken_at_utc_day: null,
                        is_video: false,
                        is_live: false,
                        is_motion: false,
                        // Smoke exercises criterion 9: show the RAW toggle.
                        is_raw: true,
                        is_screenshot: false,
                        filename: "IMG_0001.CR2",
                        taken_at_utc: null,
                        gps: null,
                        device: null,
                        lens: null,
                        exif_json: null,
                    };
                case "list_albums":
                    return state.albums;
                case "create_album":
                    state.albums.push({
                        id: state.albums.length + 1,
                        name: args.name,
                        kind: "album",
                        member_count: 0,
                        has_password: !!args.password,
                        unlocked: !args.password,
                        hidden: false,
                    });
                    return state.albums.length;
                case "add_to_album":
                    return null;
                case "album_page":
                    return { entries: state.assets, next_cursor: null };
                case "export_album":
                    return { files_written: 1, bytes_written: 2048, xmp_written: 1, skipped: 0 };
                case "lock":
                    state.session = null;
                    return null;
                case "ml_status":
                    return {
                        models_available: false,
                        runtime_loaded: false,
                        execution_provider: "disabled",
                        pending: 0,
                        running: 0,
                        done: 0,
                        failed: 0,
                    };
                case "ml_models_enabled":
                    return false;
                case "ml_reindex":
                    return { embed_queued: 0, detect_queued: 0, assets_touched: 0 };
                case "peer_my_ticket":
                    return {
                        base32: "aeaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        my_node_id_hex:
                            "0000000000000000000000000000000000000000000000000000000000000000",
                        created_at: 0,
                    };
                case "peer_accept_ticket":
                    return {
                        node_id_hex:
                            "1111111111111111111111111111111111111111111111111111111111111111",
                        identity_pub_hex:
                            "2222222222222222222222222222222222222222222222222222222222222222",
                        relay_url: null,
                        added_at: 0,
                    };
                case "peer_list":
                    return [];
                case "peer_forget":
                    return true;
                default:
                    throw new Error(`mock: unhandled ${cmd}`);
            }
        };
    });

    // Monkeypatch the Tauri invoke module before the app loads.
    await page.addInitScript(() => {
        const mod = "/node_modules/@tauri-apps/api/core.js";
        // The Tauri plugin expects window.__TAURI_INTERNALS__; mock it.
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (window as any).__TAURI_INTERNALS__ = {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            invoke: (cmd: string, args: any) => (window as any).__MV_MOCK_IPC__(cmd, args),
            metadata: { windows: [] },
        };
        void mod;
    });

    await page.goto("/");

    // Create user flow.
    await page.getByLabel("Username").fill("alice");
    await page.getByLabel("Password", { exact: true }).fill("very-long-pw-xyz");
    await page.getByLabel("Confirm password").fill("very-long-pw-xyz");
    await page.getByRole("button", { name: "Create vault" }).click();

    // Timeline loads.
    await expect(page.getByRole("button", { name: "Timeline" })).toHaveClass(/active/);

    // Add a source.
    await page.getByRole("button", { name: "Sources" }).click();
    await page.getByLabel("Name").fill("Test");
    // `getByLabel("Folder")` matches both the source-folder input and the
    // Adapter <select> (whose options include "Generic folder"). Use the
    // exact role+name locator so strict mode picks just the path input.
    await page
        .getByRole("textbox", { name: "Folder", exact: true })
        .fill("/tmp/nonexistent");
    await page.getByRole("button", { name: "Add source" }).click();
    await expect(page.getByText("Test", { exact: true })).toBeVisible();

    // Back to timeline, open asset detail.
    await page.getByRole("button", { name: "Timeline" }).click();
    await page.locator(".timeline-cell").first().click();
    await expect(page.getByText("IMG_0001.CR2")).toBeVisible();
    // Criterion 9: RAW badge + toggle visible on RAW assets.
    await expect(page.getByText("RAW", { exact: true })).toBeVisible();
    await expect(page.getByTestId("raw-toggle")).toBeVisible();
    await page.getByTestId("raw-toggle").click();
    await expect(page.getByTestId("raw-toggle")).toHaveText("Show JPEG preview");

    // Create an album, export it.
    await page.getByRole("button", { name: "← Back" }).click();
    await page.getByRole("button", { name: "Albums" }).click();
    await page.getByPlaceholder("New album name").fill("Smoke Album");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByText("Smoke Album")).toBeVisible();

    // Peers (Phase 3.1): generate a ticket and confirm the base32 renders.
    await page.getByRole("button", { name: "Peers" }).click();
    await page.getByRole("button", { name: "Generate ticket" }).click();
    await expect(page.getByTestId("peers-ticket-card")).toBeVisible();
    await expect(page.getByTestId("peers-ticket-base32")).toHaveValue(/aeaa/);
});
