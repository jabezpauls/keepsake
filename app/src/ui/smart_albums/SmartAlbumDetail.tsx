import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../ipc";
import { CollectionView } from "../timeline/Timeline";
import { useSession } from "../../state/session";

export default function SmartAlbumDetail({ id, name }: { id: number; name: string }) {
    const setView = useSession((s) => s.setView);
    const queryClient = useQueryClient();
    const [busy, setBusy] = useState(false);
    const [status, setStatus] = useState<string | null>(null);

    const refresh = async () => {
        setBusy(true);
        setStatus(null);
        try {
            const n = await api.refreshSmartAlbum(id);
            setStatus(`Snapshot updated: ${n} items`);
            await queryClient.invalidateQueries({ queryKey: ["smart-album-page", id] });
            await queryClient.invalidateQueries({ queryKey: ["smart-albums"] });
        } catch (e) {
            setStatus(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="album-detail">
            <nav className="album-detail-nav">
                <button onClick={() => setView({ kind: "smart_albums" })}>← Smart Albums</button>
                <h2>{name}</h2>
                <span className="spacer" />
                <button onClick={refresh} disabled={busy}>
                    {busy ? "Refreshing…" : "Refresh"}
                </button>
            </nav>
            {status && <p className="muted">{status}</p>}
            <CollectionView
                queryKey={["smart-album-page", id]}
                fetchPage={(cursor) => api.smartAlbumPage(id, cursor, 120)}
            />
        </div>
    );
}
