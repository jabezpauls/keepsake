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
import PersonDetail from "../people/PersonDetail";
import Duplicates from "../duplicates/Duplicates";
import Peers from "../peers/Peers";
import Trips from "../trips/Trips";
import Memories from "../memories/Memories";

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
                {navButton("trips", "Trips")}
                {navButton("memories", "Memories")}
                {navButton("duplicates", "Duplicates")}
                {navButton(
                    "albums",
                    "Albums",
                    view.kind === "albums" || view.kind === "album",
                )}
                {navButton("sources", "Sources")}
                {navButton("peers", "Peers")}
                <span className="spacer" />
                {hiddenUnlocked && <span className="hidden-badge">hidden</span>}
                {ml && <MlBadge status={ml} />}
                {ml?.models_available && <ReindexButton />}
                <button onClick={lock}>Lock</button>
            </nav>
            <section className="view-host">
                {view.kind === "timeline" && <Timeline />}
                {view.kind === "sources" && <Sources />}
                {view.kind === "albums" && <Albums />}
                {view.kind === "album" && <AlbumDetail id={view.id} name={view.name} />}
                {view.kind === "asset" && (
                    <AssetDetail
                        id={view.id}
                        back={view.back}
                        neighbors={view.neighbors}
                        index={view.index}
                    />
                )}
                {view.kind === "search" && <Search />}
                {view.kind === "map" && <MapView />}
                {view.kind === "people" && <People />}
                {view.kind === "person" && (
                    <PersonDetail id={view.id} name={view.name} />
                )}
                {view.kind === "duplicates" && <Duplicates />}
                {view.kind === "peers" && <Peers />}
                {view.kind === "trips" && <Trips />}
                {view.kind === "memories" && <Memories />}
            </section>
        </div>
    );
}

function ReindexButton() {
    const [busy, setBusy] = useState(false);
    const [last, setLast] = useState<string | null>(null);
    const run = async () => {
        setBusy(true);
        try {
            const r = await api.mlReindex();
            setLast(
                r.embed_queued + r.detect_queued === 0
                    ? "library already reindexed"
                    : `queued ${r.embed_queued + r.detect_queued} jobs across ${r.assets_touched} assets`,
            );
        } catch (err) {
            setLast(String(err));
        } finally {
            setBusy(false);
        }
    };
    return (
        <button
            className="reindex-btn"
            onClick={run}
            disabled={busy}
            title={last ?? "Enqueue ML jobs for assets that haven't been processed"}
        >
            {busy ? "Reindexing…" : "Reindex ML"}
        </button>
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
