import { useEffect } from "react";
import { useSession } from "../../state/session";
import { api } from "../../ipc";
import Timeline from "../timeline/Timeline";
import Sources from "../sources/Sources";
import Albums from "../albums/Albums";
import AlbumDetail from "../albums/AlbumDetail";
import AssetDetail from "../timeline/AssetDetail";
import Search from "../search/Search";
import MapView from "../map/MapView";
import People from "../people/People";
import Duplicates from "../duplicates/Duplicates";

export default function Shell() {
    const view = useSession((s) => s.view);
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const reset = useSession((s) => s.reset);

    const lock = async () => {
        await api.lock();
        reset();
    };

    // Keyboard shortcut: `/` focuses the search view.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
                return;
            }
            if (e.key === "/") {
                e.preventDefault();
                setView({ kind: "search" });
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [setView]);

    const navButton = (kind: typeof view.kind, label: string, isActive?: boolean) => (
        <button
            className={(isActive ?? view.kind === kind) ? "active" : ""}
            onClick={() => setView({ kind } as never)}
        >
            {label}
        </button>
    );

    return (
        <div className="shell">
            <nav className="top-nav">
                {navButton("timeline", "Timeline")}
                {navButton("search", "Search")}
                {navButton("map", "Map")}
                {navButton("people", "People")}
                {navButton("duplicates", "Duplicates")}
                {navButton(
                    "albums",
                    "Albums",
                    view.kind === "albums" || view.kind === "album",
                )}
                {navButton("sources", "Sources")}
                <span className="spacer" />
                {hiddenUnlocked && <span className="hidden-badge">hidden</span>}
                <button onClick={lock}>Lock</button>
            </nav>
            <section className="view-host">
                {view.kind === "timeline" && <Timeline />}
                {view.kind === "sources" && <Sources />}
                {view.kind === "albums" && <Albums />}
                {view.kind === "album" && <AlbumDetail id={view.id} name={view.name} />}
                {view.kind === "asset" && <AssetDetail id={view.id} back={view.back} />}
                {view.kind === "search" && <Search />}
                {view.kind === "map" && <MapView />}
                {view.kind === "people" && <People />}
                {view.kind === "duplicates" && <Duplicates />}
            </section>
        </div>
    );
}
