import { useEffect, useState } from "react";
import { useSession } from "../../state/session";
import { api } from "../../ipc";
import type { MlStatus } from "../../bindings/MlStatus";
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

    // Poll ML status every 4s. Cheap: just hits the plaintext job-count query.
    const [ml, setMl] = useState<MlStatus | null>(null);
    useEffect(() => {
        let alive = true;
        const tick = async () => {
            try {
                const s = await api.mlStatus();
                if (alive) setMl(s);
            } catch {
                // ignore — UI just keeps the last known state
            }
        };
        tick();
        const h = window.setInterval(tick, 4000);
        return () => {
            alive = false;
            window.clearInterval(h);
        };
    }, []);

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
                {ml && <MlBadge status={ml} />}
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

function MlBadge({ status }: { status: MlStatus }) {
    if (!status.models_available) {
        return (
            <span className="ml-badge ml-badge--off" title="Build without --features ml-models">
                ML off
            </span>
        );
    }
    if (!status.runtime_loaded) {
        return (
            <span
                className="ml-badge ml-badge--missing"
                title="Models feature on, but weights not loaded. Run scripts/download_models.sh."
            >
                ML — no weights
            </span>
        );
    }
    const queued = status.pending + status.running;
    if (queued > 0) {
        return (
            <span
                className="ml-badge ml-badge--running"
                title={`${status.pending} pending · ${status.running} running · ${status.done} done · ${status.failed} failed`}
            >
                ML {status.execution_provider} · {queued} queued
            </span>
        );
    }
    return (
        <span
            className="ml-badge ml-badge--idle"
            title={`${status.done} done · ${status.failed} failed`}
        >
            ML {status.execution_provider} · idle
        </span>
    );
}
