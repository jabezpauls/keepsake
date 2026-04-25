import { test, expect } from "@playwright/test";

// Phase 2 cmdk + sidebar smoke. Lighter than smoke.spec.ts —
// the goal is to confirm the new shell loads, the sidebar zones
// work, and ⌘K opens / navigates / selects.
//
// Mocks are stubbed minimally; we just need user-creation + a
// timeline-page response so the Library zone renders without errors.

test("new shell: sidebar zones + cmd-K navigation", async ({ page }) => {
    await page.addInitScript(() => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const state: any = { userExists: false, session: null };
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (window as any).__MV_MOCK_IPC__ = async (cmd: string, args: any) => {
            switch (cmd) {
                case "user_exists":
                    return state.userExists;
                case "list_users":
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
                    return [];
                case "list_albums":
                    return [];
                case "timeline_page":
                    return { entries: [], next_cursor: null };
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
                case "list_people":
                case "list_trips":
                case "list_smart_albums":
                case "memories_on_this_day":
                case "memories_year_in_photos":
                case "memories_person_year":
                case "near_dup_list":
                    return [];
                case "lock":
                    state.session = null;
                    return null;
                default:
                    return null;
            }
        };
    });

    await page.addInitScript(() => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (window as any).__TAURI_INTERNALS__ = {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            invoke: (cmd: string, args: any) =>
                (window as any).__MV_MOCK_IPC__(cmd, args),
            metadata: { windows: [] },
        };
    });

    await page.goto("/");

    // Create user — Unlock screen is unchanged in Phase 2.
    await page.getByLabel("Username").fill("alice");
    await page.getByLabel("Password", { exact: true }).fill("very-long-pw-xyz");
    await page.getByLabel("Confirm password").fill("very-long-pw-xyz");
    await page.getByRole("button", { name: "Create vault" }).click();

    // Sidebar exists and Library is the active zone by default.
    await expect(page.locator(".kp-sidebar")).toBeVisible();
    await expect(
        page.locator('.kp-sidebar-zone[data-active="true"]'),
    ).toContainText("Library");

    // Click Albums zone.
    await page.getByRole("button", { name: "Albums", exact: true }).click();
    await expect(
        page.locator('.kp-sidebar-zone[data-active="true"]'),
    ).toContainText("Albums");

    // Open the command palette via ⌘K.
    await page.keyboard.press("Control+k");
    await expect(page.locator(".kp-cmdk-panel")).toBeVisible();

    // Type "Search" and select the first match.
    await page.locator(".kp-cmdk-input").fill("search");
    await page.keyboard.press("Enter");

    // Now Search zone should be active.
    await expect(page.locator(".kp-cmdk-panel")).toBeHidden();
    await expect(
        page.locator('.kp-sidebar-zone[data-active="true"]'),
    ).toContainText("Search");

    // Esc closes the palette mid-typing.
    await page.keyboard.press("Control+k");
    await expect(page.locator(".kp-cmdk-panel")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".kp-cmdk-panel")).toBeHidden();
});
