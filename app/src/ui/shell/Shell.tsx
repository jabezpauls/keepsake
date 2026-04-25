import { useEffect, useMemo, useState } from "react";
import { useSession, View } from "../../state/session";
import { api } from "../../ipc";
import type { MlStatus } from "../../bindings/MlStatus";
import { Breadcrumb, BreadcrumbItem, ToastProvider, TooltipProvider } from "../../components";
import { Sidebar } from "./Sidebar";
import { SettingsStub } from "./SettingsStub";
import LegacyShell from "./LegacyShell";
import { CommandPalette } from "../cmdk/CommandPalette";

import LibraryRouter from "../library/LibraryRouter";
import ForYouPlaceholder from "../for_you/ForYouPlaceholder";
import Sources from "../sources/Sources";
import Albums from "../albums/Albums";
import AlbumDetail from "../albums/AlbumDetail";
import AssetDetail from "../library/AssetDetail";
import Search from "../search/Search";
import MapView from "../map/MapView";
import People from "../people/People";
import PersonDetail from "../people/PersonDetail";
import Duplicates from "../duplicates/Duplicates";
import Peers from "../peers/Peers";
import Trips from "../trips/Trips";
import Memories from "../memories/Memories";
import SmartAlbums from "../smart_albums/SmartAlbums";
import SmartAlbumDetail from "../smart_albums/SmartAlbumDetail";
import Pets from "../pets/Pets";
import ModelDownloadWizard from "../ml/ModelDownloadWizard";
import "./shell.css";

// New IA shell — Phase 2.
//
// Layout: persistent left sidebar + content area with breadcrumb.
// The sidebar exposes the four zones (Library / For You / Albums /
// Search) and a Pinned section for the legacy single-purpose screens
// until Phase 6 absorbs them. ⌘K opens the command palette; the gear
// in the sidebar footer opens a Settings sheet (full content lands
// in Phase 8).
//
// Backwards-compat: every existing `view.kind` still routes to its
// legacy screen, so feature work in earlier phases continues to land
// without redesign churn. The legacy top-nav itself remains reachable
// via `?legacy=1` for A/B comparison.
export default function Shell() {
    // Dispatch on the legacy flag before any hooks fire — keeps
    // rules-of-hooks happy because the new shell's hook set differs
    // from LegacyShell's.
    if (
        typeof window !== "undefined" &&
        new URLSearchParams(window.location.search).get("legacy") === "1"
    ) {
        return <LegacyShell />;
    }
    return <NewShell />;
}

function NewShell() {
    const view = useSession((s) => s.view);
    const backstack = useSession((s) => s.backstack);
    const setView = useSession((s) => s.setView);
    const popView = useSession((s) => s.popView);
    const reset = useSession((s) => s.reset);

    const lock = async () => {
        await api.lock();
        reset();
    };

    // ML status + first-run wizard — same behaviour as the legacy shell.
    const [ml, setMl] = useState<MlStatus | null>(null);
    useEffect(() => {
        let alive = true;
        const tick = async () => {
            try {
                const s = await api.mlStatus();
                if (alive) setMl(s);
            } catch {
                // ignore
            }
        };
        void tick();
        const h = window.setInterval(tick, 4000);
        return () => {
            alive = false;
            window.clearInterval(h);
        };
    }, []);

    const [wizardOpen, setWizardOpen] = useState(false);
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [paletteOpen, setPaletteOpen] = useState(false);

    useEffect(() => {
        if (!ml) return;
        if (!ml.models_available || ml.runtime_loaded) return;
        if (sessionStorage.getItem("mv-wizard-prompted") === "1") return;
        sessionStorage.setItem("mv-wizard-prompted", "1");
        setWizardOpen(true);
    }, [ml]);

    // Keyboard shortcuts.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            const isInput =
                e.target instanceof HTMLInputElement ||
                e.target instanceof HTMLTextAreaElement;
            if (!isInput && e.key === "/") {
                e.preventDefault();
                setView({ kind: "search" });
            }
            if (e.key === "Escape" && backstack.length > 1) {
                e.preventDefault();
                popView();
            }
            if ((e.key === "," && (e.metaKey || e.ctrlKey)) && !isInput) {
                e.preventDefault();
                setSettingsOpen(true);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [setView, popView, backstack.length]);

    // Long-press on the sidebar logo is reserved for the hidden-vault
    // entry surface (Phase 8 wires it to a lock-and-reunlock flow). For
    // Phase 2 we just no-op here so the affordance stays discoverable
    // without breaking anything.
    const onLogoLongPress = () => {
        // Intentionally empty until Phase 8 — see plans/wise-strolling-otter.md.
    };

    const breadcrumb = useMemo<BreadcrumbItem[]>(
        () => buildBreadcrumb(backstack, popView),
        [backstack, popView],
    );

    return (
        <TooltipProvider>
            <ToastProvider>
                <div className="kp-shell">
                    <Sidebar
                        onOpenCommand={() => setPaletteOpen(true)}
                        onOpenSettings={() => setSettingsOpen(true)}
                        onLock={lock}
                        onLogoLongPress={onLogoLongPress}
                    />
                    <main className="kp-shell-content">
                        {breadcrumb.length > 1 && (
                            <div className="kp-shell-breadcrumb">
                                <Breadcrumb items={breadcrumb} />
                            </div>
                        )}
                        <section className="kp-shell-view">
                            <ViewHost view={view} />
                        </section>
                    </main>

                    <SettingsStub
                        open={settingsOpen}
                        onOpenChange={setSettingsOpen}
                        onOpenWizard={() => setWizardOpen(true)}
                    />
                    <CommandPalette
                        open={paletteOpen}
                        onOpenChange={setPaletteOpen}
                        onOpenSettings={() => setSettingsOpen(true)}
                        onOpenWizard={() => setWizardOpen(true)}
                    />
                    {wizardOpen && (
                        <ModelDownloadWizard onClose={() => setWizardOpen(false)} />
                    )}
                </div>
            </ToastProvider>
        </TooltipProvider>
    );
}

// Render the right component for the current view. Every legacy view
// kind is preserved so this phase doesn't break existing screens; new
// kinds (library, for-you, place, settings) get their own routes too.
function ViewHost({ view }: { view: View }) {
    switch (view.kind) {
        case "library":
            return <LibraryRouter />;
        case "for-you":
            return <ForYouPlaceholder />;
        case "timeline":
            // Timeline still maps directly until Phase 3 collapses it
            // into the new Library component.
            return <LibraryRouter />;
        case "sources":
            return <Sources />;
        case "albums":
            return <Albums />;
        case "album":
            return <AlbumDetail id={view.id} name={view.name} />;
        case "asset":
            return (
                <AssetDetail
                    id={view.id}
                    back={view.back}
                    neighbors={view.neighbors}
                    index={view.index}
                />
            );
        case "search":
            return <Search />;
        case "map":
            return <MapView />;
        case "people":
            return <People />;
        case "person":
            return <PersonDetail id={view.id} name={view.name} />;
        case "place":
            // Phase 4 ships the real Place screen — for now show a
            // friendly placeholder so chips that point here don't break.
            return <PlacePlaceholder name={view.name} />;
        case "duplicates":
            return <Duplicates />;
        case "peers":
            return <Peers />;
        case "trips":
            return <Trips />;
        case "memories":
            return <Memories />;
        case "smart_albums":
            return <SmartAlbums />;
        case "smart_album":
            return <SmartAlbumDetail id={view.id} name={view.name} />;
        case "pets":
            return <Pets />;
        case "settings":
            // Settings live in a Sheet, so the actual section is opened
            // via setSettingsOpen + the SettingsStub component, not via
            // this view kind. If we land here directly (e.g. a chip
            // routes a peer to settings), fall back to Peers for now.
            return <Peers />;
    }
}

function PlacePlaceholder({ name }: { name: string }) {
    return (
        <div style={{ padding: "var(--space-8) var(--space-6)" }}>
            <h1 style={{ font: "var(--font-display)", margin: 0 }}>{name}</h1>
            <p
                style={{
                    color: "var(--color-text-secondary)",
                    marginTop: "var(--space-3)",
                }}
            >
                Place screen lands in Phase 4. Backend command{" "}
                <code className="kp-mono">list_places</code> + cross-link
                chips will populate this view.
            </p>
        </div>
    );
}

// Walk the backstack and produce breadcrumb items. Each frame except
// the last is clickable — clicking pops the stack down to that frame.
function buildBreadcrumb(stack: View[], popView: () => void): BreadcrumbItem[] {
    if (stack.length <= 1) return [];

    const items: BreadcrumbItem[] = [];
    for (let i = 0; i < stack.length; i += 1) {
        const v = stack[i];
        const label = labelForView(v);
        const popsToHere = stack.length - 1 - i;
        if (i === stack.length - 1) {
            items.push({ label });
        } else {
            items.push({
                label,
                onClick: () => {
                    for (let j = 0; j < popsToHere; j += 1) popView();
                },
            });
        }
    }
    return items;
}

function labelForView(v: View): string {
    switch (v.kind) {
        case "library":
        case "timeline":
            return "Library";
        case "for-you":
            return "For You";
        case "sources":
            return "Sources";
        case "albums":
            return "Albums";
        case "album":
            return v.name;
        case "asset":
            return "Photo";
        case "search":
            return "Search";
        case "map":
            return "Map";
        case "people":
            return "People";
        case "person":
            return v.name ?? "Person";
        case "place":
            return v.name;
        case "duplicates":
            return "Duplicates";
        case "peers":
            return "Peers";
        case "trips":
            return "Trips";
        case "memories":
            return "Memories";
        case "smart_albums":
            return "Smart albums";
        case "smart_album":
            return v.name;
        case "pets":
            return "Pets";
        case "settings":
            return "Settings";
    }
}
