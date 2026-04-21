import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, bytesToBlobUrl } from "../../ipc";
import type { View } from "../../state/session";
import { useSession } from "../../state/session";
import type { AlbumView } from "../../bindings/AlbumView";

interface Props {
    id: number;
    back: View;
}

export default function AssetDetail({ id, back }: Props) {
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const queryClient = useQueryClient();

    const { data: detail, isLoading } = useQuery({
        queryKey: ["asset-detail", id],
        queryFn: () => api.assetDetail(id),
    });

    const { data: albums = [] } = useQuery<AlbumView[]>({
        queryKey: ["albums", hiddenUnlocked ? "withHidden" : "plain"],
        queryFn: () => api.listAlbums(hiddenUnlocked),
    });

    const [fullUrl, setFullUrl] = useState<string | null>(null);
    useEffect(() => {
        if (!detail) return;
        let url: string | null = null;
        let cancelled = false;
        void (async () => {
            try {
                const bytes = await api.assetThumbnail(id, 1024);
                if (cancelled) return;
                url = bytesToBlobUrl(bytes, "image/webp");
                setFullUrl(url);
            } catch {
                /* fall back silently */
            }
        })();
        return () => {
            cancelled = true;
            if (url) URL.revokeObjectURL(url);
            setFullUrl(null);
        };
    }, [id, detail]);

    const addToAlbum = async (albumId: number) => {
        await api.addToAlbum(albumId, [id]);
        await queryClient.invalidateQueries({ queryKey: ["albums"] });
    };

    if (isLoading || !detail) return <div className="asset-detail-loading">Loading…</div>;

    return (
        <div className="asset-detail">
            <nav className="asset-detail-nav">
                <button onClick={() => setView(back)}>← Back</button>
                <span className="filename">{detail.filename}</span>
            </nav>
            <div className="asset-detail-body">
                <div className="asset-image-wrap">
                    {fullUrl ? <img src={fullUrl} alt={detail.filename} /> : <div className="thumb-loading" />}
                </div>
                <aside className="asset-sidebar">
                    <Row label="Type" value={detail.mime} />
                    <Row label="Bytes" value={detail.bytes.toLocaleString()} />
                    {detail.width && detail.height && (
                        <Row label="Dimensions" value={`${detail.width} × ${detail.height}`} />
                    )}
                    {detail.duration_ms != null && (
                        <Row label="Duration" value={`${(detail.duration_ms / 1000).toFixed(1)}s`} />
                    )}
                    {detail.taken_at_utc && <Row label="Taken" value={detail.taken_at_utc} />}
                    {detail.device && <Row label="Device" value={detail.device} />}
                    {detail.lens && <Row label="Lens" value={detail.lens} />}
                    {detail.gps && (
                        <Row
                            label="GPS"
                            value={`${detail.gps.lat.toFixed(5)}, ${detail.gps.lon.toFixed(5)}`}
                        />
                    )}
                    <div className="flags">
                        {detail.is_live && <Flag text="Live" />}
                        {detail.is_motion && <Flag text="Motion" />}
                        {detail.is_raw && <Flag text="RAW" />}
                        {detail.is_screenshot && <Flag text="Screenshot" />}
                        {detail.is_video && <Flag text="Video" />}
                    </div>

                    <div className="add-to-album">
                        <strong>Add to album</strong>
                        {albums.length === 0 && <p>(no albums yet)</p>}
                        {albums.map((a) => (
                            <button key={a.id} onClick={() => addToAlbum(a.id)}>
                                + {a.name}
                            </button>
                        ))}
                    </div>

                    {detail.exif_json && (
                        <details className="exif-panel">
                            <summary>Full EXIF</summary>
                            <pre>{formatExifJson(detail.exif_json)}</pre>
                        </details>
                    )}
                </aside>
            </div>
        </div>
    );
}

function Row({ label, value }: { label: string; value: string }) {
    return (
        <div className="asset-row">
            <span className="label">{label}</span>
            <span className="value">{value}</span>
        </div>
    );
}

function Flag({ text }: { text: string }) {
    return <span className="flag">{text}</span>;
}

function formatExifJson(raw: string): string {
    try {
        return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
        return raw;
    }
}
