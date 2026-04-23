# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: smoke.spec.ts >> create user → add source → timeline → album export
- Location: tests/e2e/smoke.spec.ts:14:1

# Error details

```
Error: locator.fill: Error: strict mode violation: getByLabel('Folder') resolved to 2 elements:
    1) <input value=""/> aka getByRole('textbox', { name: 'Folder Browse…' })
    2) <select>…</select> aka getByLabel('AdapterGeneric folderiPhone (')

Call log:
  - waiting for getByLabel('Folder')

```

# Page snapshot

```yaml
- generic [ref=e3]:
  - navigation [ref=e4]:
    - button "Timeline" [ref=e5] [cursor=pointer]
    - button "Search" [ref=e6] [cursor=pointer]
    - button "Map" [ref=e7] [cursor=pointer]
    - button "People" [ref=e8] [cursor=pointer]
    - button "Duplicates" [ref=e9] [cursor=pointer]
    - button "Albums" [ref=e10] [cursor=pointer]
    - button "Sources" [ref=e11] [cursor=pointer]
    - button "Lock" [ref=e12] [cursor=pointer]
  - generic [ref=e14]:
    - heading "Sources" [level=2] [ref=e15]
    - generic [ref=e16]:
      - heading "Add a source" [level=3] [ref=e17]
      - generic [ref=e18]:
        - generic [ref=e19]: Name
        - textbox "Name" [active] [ref=e20]: Test
      - generic [ref=e21]:
        - generic [ref=e22]: Folder
        - generic [ref=e23]:
          - textbox "Folder Browse…" [ref=e24]
          - button "Browse…" [ref=e25] [cursor=pointer]
      - generic [ref=e26]:
        - generic [ref=e27]: Adapter
        - combobox "Adapter" [ref=e28]:
          - option "Generic folder" [selected]
          - option "iPhone (DCIM)"
          - option "Google Takeout"
      - generic [ref=e29]:
        - checkbox "Link only (don't copy into vault)" [ref=e30]
        - generic [ref=e31]: Link only (don't copy into vault)
      - button "Add source" [ref=e32] [cursor=pointer]
    - paragraph [ref=e33]: (no sources yet)
    - list
```

# Test source

```ts
  40  |                     return state.sources;
  41  |                 case "add_source":
  42  |                     state.sources.push({
  43  |                         id: state.sources.length + 1,
  44  |                         name: args.name,
  45  |                         root_path: args.root,
  46  |                         adapter_kind: args.adapter,
  47  |                         linked_only: args.linkedOnly,
  48  |                         bytes_total: 2048,
  49  |                         file_count: 1,
  50  |                         imported_at: 0,
  51  |                     });
  52  |                     state.assets.push({ id: 1, mime: "image/jpeg", is_video: false, is_live: false });
  53  |                     return state.sources.length;
  54  |                 case "ingest_status":
  55  |                     return {
  56  |                         source_id: args.sourceId,
  57  |                         state: { state: "done", inserted: 1, deduped: 0, skipped: 0, errors: 0 },
  58  |                     };
  59  |                 case "timeline_page":
  60  |                     return { entries: state.assets, next_cursor: null };
  61  |                 case "asset_thumbnail":
  62  |                 case "asset_original":
  63  |                     return [];
  64  |                 case "asset_detail":
  65  |                     return {
  66  |                         id: args.id,
  67  |                         mime: "image/jpeg",
  68  |                         bytes: 2048,
  69  |                         width: 1920,
  70  |                         height: 1080,
  71  |                         duration_ms: null,
  72  |                         taken_at_utc_day: null,
  73  |                         is_video: false,
  74  |                         is_live: false,
  75  |                         is_motion: false,
  76  |                         is_raw: false,
  77  |                         is_screenshot: false,
  78  |                         filename: "IMG_0001.JPG",
  79  |                         taken_at_utc: null,
  80  |                         gps: null,
  81  |                         device: null,
  82  |                         lens: null,
  83  |                         exif_json: null,
  84  |                     };
  85  |                 case "list_albums":
  86  |                     return state.albums;
  87  |                 case "create_album":
  88  |                     state.albums.push({
  89  |                         id: state.albums.length + 1,
  90  |                         name: args.name,
  91  |                         kind: "album",
  92  |                         member_count: 0,
  93  |                         has_password: !!args.password,
  94  |                         unlocked: !args.password,
  95  |                         hidden: false,
  96  |                     });
  97  |                     return state.albums.length;
  98  |                 case "add_to_album":
  99  |                     return null;
  100 |                 case "album_page":
  101 |                     return { entries: state.assets, next_cursor: null };
  102 |                 case "export_album":
  103 |                     return { files_written: 1, bytes_written: 2048, xmp_written: 1, skipped: 0 };
  104 |                 case "lock":
  105 |                     state.session = null;
  106 |                     return null;
  107 |                 default:
  108 |                     throw new Error(`mock: unhandled ${cmd}`);
  109 |             }
  110 |         };
  111 |     });
  112 | 
  113 |     // Monkeypatch the Tauri invoke module before the app loads.
  114 |     await page.addInitScript(() => {
  115 |         const mod = "/node_modules/@tauri-apps/api/core.js";
  116 |         // The Tauri plugin expects window.__TAURI_INTERNALS__; mock it.
  117 |         // eslint-disable-next-line @typescript-eslint/no-explicit-any
  118 |         (window as any).__TAURI_INTERNALS__ = {
  119 |             // eslint-disable-next-line @typescript-eslint/no-explicit-any
  120 |             invoke: (cmd: string, args: any) => (window as any).__MV_MOCK_IPC__(cmd, args),
  121 |             metadata: { windows: [] },
  122 |         };
  123 |         void mod;
  124 |     });
  125 | 
  126 |     await page.goto("/");
  127 | 
  128 |     // Create user flow.
  129 |     await page.getByLabel("Username").fill("alice");
  130 |     await page.getByLabel("Password", { exact: true }).fill("very-long-pw-xyz");
  131 |     await page.getByLabel("Confirm password").fill("very-long-pw-xyz");
  132 |     await page.getByRole("button", { name: "Create vault" }).click();
  133 | 
  134 |     // Timeline loads.
  135 |     await expect(page.getByRole("button", { name: "Timeline" })).toHaveClass(/active/);
  136 | 
  137 |     // Add a source.
  138 |     await page.getByRole("button", { name: "Sources" }).click();
  139 |     await page.getByLabel("Name").fill("Test");
> 140 |     await page.getByLabel("Folder").fill("/tmp/nonexistent");
      |                                     ^ Error: locator.fill: Error: strict mode violation: getByLabel('Folder') resolved to 2 elements:
  141 |     await page.getByRole("button", { name: "Add source" }).click();
  142 |     await expect(page.getByText("Test", { exact: true })).toBeVisible();
  143 | 
  144 |     // Back to timeline, open asset detail.
  145 |     await page.getByRole("button", { name: "Timeline" }).click();
  146 |     await page.locator(".timeline-cell").first().click();
  147 |     await expect(page.getByText("IMG_0001.JPG")).toBeVisible();
  148 | 
  149 |     // Create an album, export it.
  150 |     await page.getByRole("button", { name: "← Back" }).click();
  151 |     await page.getByRole("button", { name: "Albums" }).click();
  152 |     await page.getByPlaceholder("New album name").fill("Smoke Album");
  153 |     await page.getByRole("button", { name: "Create" }).click();
  154 |     await expect(page.getByText("Smoke Album")).toBeVisible();
  155 | });
  156 | 
```