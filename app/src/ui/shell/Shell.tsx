import { useSession } from "../../state/session";
import { api } from "../../ipc";
import Timeline from "../timeline/Timeline";
import Sources from "../sources/Sources";
import Albums from "../albums/Albums";
import AlbumDetail from "../albums/AlbumDetail";
import AssetDetail from "../timeline/AssetDetail";

export default function Shell() {
    const view = useSession((s) => s.view);
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const reset = useSession((s) => s.reset);

    const lock = async () => {
        await api.lock();
        reset();
    };

    return (
        <div className="shell">
            <nav className="top-nav">
                <button
                    className={view.kind === "timeline" ? "active" : ""}
                    onClick={() => setView({ kind: "timeline" })}
                >
                    Timeline
                </button>
                <button
                    className={view.kind === "albums" || view.kind === "album" ? "active" : ""}
                    onClick={() => setView({ kind: "albums" })}
                >
                    Albums
                </button>
                <button
                    className={view.kind === "sources" ? "active" : ""}
                    onClick={() => setView({ kind: "sources" })}
                >
                    Sources
                </button>
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
            </section>
        </div>
    );
}
