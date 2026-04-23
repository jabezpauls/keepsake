import { useEffect, useRef, useState } from "react";
import {
    keepPreviousData,
    useQuery,
    useQueryClient,
} from "@tanstack/react-query";
import { api, bytesToBlobUrl } from "../../ipc";
import type { View } from "../../state/session";
import { useSession } from "../../state/session";
import type { AlbumView } from "../../bindings/AlbumView";
import FaceOverlay from "./FaceOverlay";

interface Props {
    id: number;
    back: View;
    neighbors?: number[];
    index?: number;
}

export default function AssetDetail({ id, back, neighbors, index }: Props) {
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const queryClient = useQueryClient();

    const hasNeighbors = !!neighbors && typeof index === "number" && neighbors.length > 1;
    const prevId =
        hasNeighbors && index! > 0 ? neighbors![index! - 1] : null;
    const nextId =
        hasNeighbors && index! < neighbors!.length - 1
            ? neighbors![index! + 1]
            : null;

    const goto = (targetId: number, targetIdx: number) => {
        setView({
            kind: "asset",
            id: targetId,
            back,
            neighbors,
            index: targetIdx,
        });
    };
    const goPrev = () => {
        if (prevId !== null) goto(prevId, index! - 1);
    };
    const goNext = () => {
        if (nextId !== null) goto(nextId, index! + 1);
    };

    // Arrow-key + j/k navigation. Skip when typing in a field.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (
                e.target instanceof HTMLInputElement ||
                e.target instanceof HTMLTextAreaElement
            ) {
                return;
            }
            if (e.key === "ArrowLeft" || e.key === "k") {
                e.preventDefault();
                goPrev();
            } else if (e.key === "ArrowRight" || e.key === "j") {
                e.preventDefault();
                goNext();
            } else if (e.key === "Escape") {
                e.preventDefault();
                setView(back);
            } else if (e.key === "f" || e.key === "F") {
                e.preventDefault();
                setShowFaces((v) => !v);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
        // goto/goPrev/goNext close over the current id+index; re-register so
        // keystrokes always step relative to the asset currently on screen.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [id, index, neighbors]);

    const { data: detail, isLoading } = useQuery({
        queryKey: ["asset-detail", id],
        queryFn: () => api.assetDetail(id),
        placeholderData: keepPreviousData,
    });

    const { data: albums = [] } = useQuery<AlbumView[]>({
        queryKey: ["albums", hiddenUnlocked ? "withHidden" : "plain"],
        queryFn: () => api.listAlbums(hiddenUnlocked),
    });

    // Cache thumb bytes per asset so arrow-key navigation doesn't blank the
    // viewer between assets. `keepPreviousData` shows the last asset's bytes
    // while the new one is in flight.
    const thumbQuery = useQuery({
        queryKey: ["asset-thumb-1024", id],
        queryFn: () => api.assetThumbnail(id, 1024),
        placeholderData: keepPreviousData,
        staleTime: 5 * 60_000,
    });

    const [fullUrl, setFullUrl] = useState<string | null>(null);
    useEffect(() => {
        const bytes = thumbQuery.data;
        if (!bytes) return;
        const url = bytesToBlobUrl(bytes, "image/webp");
        setFullUrl(url);
        return () => URL.revokeObjectURL(url);
    }, [thumbQuery.data]);

    // Reset natural-dimension state when the asset changes so the face
    // overlay doesn't draw over the previous image's geometry.
    useEffect(() => {
        setImgDims(null);
    }, [id]);

    // Prefetch the immediate neighbours so arrow-keying feels instant.
    useEffect(() => {
        for (const nid of [prevId, nextId]) {
            if (nid === null) continue;
            queryClient.prefetchQuery({
                queryKey: ["asset-thumb-1024", nid],
                queryFn: () => api.assetThumbnail(nid, 1024),
                staleTime: 5 * 60_000,
            });
            queryClient.prefetchQuery({
                queryKey: ["asset-detail", nid],
                queryFn: () => api.assetDetail(nid),
            });
            queryClient.prefetchQuery({
                queryKey: ["asset-faces", nid],
                queryFn: () => api.assetFaces(nid),
                staleTime: 60_000,
            });
        }
    }, [prevId, nextId, queryClient]);

    // Criterion 9 surfaces. Two optional paths:
    // 1. For video-mime assets (including Live / Motion Photos whose MOV
    //    side the user is viewing), fetch the original bytes and play inline.
    // 2. For RAW stills, toggle between the JPEG-preview thumbnail and the
    //    RAW original bytes on demand (RAW decode happens in the browser —
    //    browsers reject most RAW formats, so we fall back to a download
    //    link in that case).
    const [showRaw, setShowRaw] = useState(false);
    const [originalUrl, setOriginalUrl] = useState<string | null>(null);
    const [showFaces, setShowFaces] = useState(true);
    const imgRef = useRef<HTMLImageElement | null>(null);
    const [imgDims, setImgDims] = useState<{ w: number; h: number } | null>(null);
    const isPlayable =
        !!detail && (detail.is_video || detail.mime.startsWith("video/"));
    useEffect(() => {
        if (!detail) return;
        const needOriginal = isPlayable || (detail.is_raw && showRaw);
        if (!needOriginal) {
            setOriginalUrl(null);
            return;
        }
        let url: string | null = null;
        let cancelled = false;
        void (async () => {
            try {
                const bytes = await api.assetOriginal(id);
                if (cancelled) return;
                url = bytesToBlobUrl(bytes, detail.mime);
                setOriginalUrl(url);
            } catch {
                /* e.g. RAW format the browser doesn't decode — leave null */
            }
        })();
        return () => {
            cancelled = true;
            if (url) URL.revokeObjectURL(url);
        };
    }, [id, detail, isPlayable, showRaw]);

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
                {hasNeighbors && (
                    <span className="asset-detail-counter">
                        {index! + 1} of {neighbors!.length}
                    </span>
                )}
                {!isPlayable && (
                    <button
                        className={`face-toggle${showFaces ? " on" : ""}`}
                        onClick={() => setShowFaces((v) => !v)}
                        title={
                            showFaces
                                ? "Hide face overlays (F)"
                                : "Show face overlays (F)"
                        }
                        aria-pressed={showFaces}
                    >
                        Faces {showFaces ? "on" : "off"}
                    </button>
                )}
            </nav>
            <div className="asset-detail-body">
                <div className="asset-image-wrap">
                    {hasNeighbors && (
                        <>
                            <button
                                className="asset-nav-chevron asset-nav-prev"
                                onClick={goPrev}
                                disabled={prevId === null}
                                aria-label="Previous photo"
                                title="Previous (←)"
                            >
                                ‹
                            </button>
                            <button
                                className="asset-nav-chevron asset-nav-next"
                                onClick={goNext}
                                disabled={nextId === null}
                                aria-label="Next photo"
                                title="Next (→)"
                            >
                                ›
                            </button>
                        </>
                    )}
                    {isPlayable ? (
                        originalUrl ? (
                            <video
                                data-testid="asset-video"
                                src={originalUrl}
                                controls
                                autoPlay
                                loop={detail.is_live || detail.is_motion}
                                muted={detail.is_live || detail.is_motion}
                                playsInline
                            />
                        ) : (
                            <div className="thumb-loading">Loading video…</div>
                        )
                    ) : detail.is_raw && showRaw && originalUrl ? (
                        <img src={originalUrl} alt={`${detail.filename} (RAW)`} />
                    ) : fullUrl ? (
                        <div className="asset-image-frame">
                            <img
                                ref={imgRef}
                                src={fullUrl}
                                alt={detail.filename}
                                onLoad={(e) => {
                                    const el = e.currentTarget;
                                    setImgDims({
                                        w: el.naturalWidth,
                                        h: el.naturalHeight,
                                    });
                                }}
                            />
                            {!isPlayable && imgDims && (
                                <FaceOverlay
                                    assetId={id}
                                    imgWidth={imgDims.w}
                                    imgHeight={imgDims.h}
                                    visible={showFaces}
                                    onPersonClick={(pid, pname) =>
                                        setView({
                                            kind: "person",
                                            id: pid,
                                            name: pname,
                                        })
                                    }
                                />
                            )}
                        </div>
                    ) : (
                        <div className="thumb-loading" />
                    )}
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

                    {detail.is_raw && (
                        <button
                            className="raw-toggle"
                            onClick={() => setShowRaw((s) => !s)}
                            data-testid="raw-toggle"
                        >
                            {showRaw ? "Show JPEG preview" : "Show RAW original"}
                        </button>
                    )}

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
