import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../ipc";
import type { ExportReport } from "../../bindings/ExportReport";
import { CollectionView } from "../timeline/Timeline";
import { useSession } from "../../state/session";

export default function AlbumDetail({ id, name }: { id: number; name: string }) {
    const setView = useSession((s) => s.setView);
    const [busy, setBusy] = useState(false);
    const [report, setReport] = useState<ExportReport | null>(null);
    const [err, setErr] = useState<string | null>(null);

    const exportAlbum = async () => {
        setErr(null);
        try {
            const picked = await openDialog({ directory: true, multiple: false });
            if (typeof picked !== "string") return;
            setBusy(true);
            const r = await api.exportAlbum(id, picked, { include_xmp: true });
            setReport(r);
        } catch (e) {
            setErr(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="album-detail">
            <nav className="album-detail-nav">
                <button onClick={() => setView({ kind: "albums" })}>← Albums</button>
                <h2>{name}</h2>
                <span className="spacer" />
                <button onClick={exportAlbum} disabled={busy}>
                    {busy ? "Exporting…" : "Export…"}
                </button>
            </nav>
            {err && <p className="error">{err}</p>}
            {report && (
                <p className="export-report">
                    Exported {report.files_written} files ({(report.bytes_written / 1024 / 1024).toFixed(1)}{" "}
                    MiB), {report.xmp_written} XMP sidecars, {report.skipped} skipped.
                </p>
            )}
            <CollectionView
                queryKey={["album-page", id]}
                fetchPage={(cursor) => api.albumPage(id, cursor, 120)}
            />
        </div>
    );
}
