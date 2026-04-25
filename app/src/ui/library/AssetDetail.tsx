import { useEffect, useMemo, useRef, useState } from "react";
import {
    keepPreviousData,
    useQuery,
    useQueryClient,
} from "@tanstack/react-query";
import { motion } from "framer-motion";
import { ArrowLeft, ChevronLeft, ChevronRight, Info, X } from "lucide-react";
import { api, bytesToBlobUrl } from "../../ipc";
import type { View } from "../../state/session";
import { useSession } from "../../state/session";
import type { AlbumView } from "../../bindings/AlbumView";
import FaceOverlay from "../timeline/FaceOverlay";
import {
    Button,
    EntityChip,
    IconButton,
    Popover,
} from "../../components";

interface Props {
    id: number;
    back: View;
    neighbors?: number[];
    index?: number;
}

// Phase 3 AssetDetail — replaces the legacy split-pane viewer with a
// full-bleed canvas + auto-fading chrome.
//
// Hero motion: the main photo wears `motion.img` with
// `layoutId={`asset-${id}`}` matching the same layoutId on the Library
// thumbnail. framer-motion bridges the transition automatically — the
// thumbnail expands smoothly into the full-bleed image when opened.
//
// Auto-fading chrome: top filename bar and bottom info chip fade out
// after 2 s of cursor idleness, return on cursor-move. `i` key (or the
// info button) opens a right-side info pane with EXIF, flags, album
// chooser, caption editor, pet flag.
//
// Backwards-compat: keyboard nav (arrows, j/k, Esc, F for faces) and
// face overlay click-to-PersonDetail preserved.
export default function AssetDetail({ id, back, neighbors, index }: Props) {
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const queryClient = useQueryClient();

    const hasNeighbors =
        !!neighbors && typeof index === "number" && neighbors.length > 1;
    const prevId = hasNeighbors && index! > 0 ? neighbors![index! - 1] : null;
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
    const goPrev = () => prevId !== null && goto(prevId, index! - 1);
    const goNext = () => nextId !== null && goto(nextId, index! + 1);

    const [showFaces, setShowFaces] = useState(true);
    const [infoOpen, setInfoOpen] = useState(false);
    const [chromeHidden, setChromeHidden] = useState(false);
    const idleTimerRef = useRef<number | null>(null);

    // Fade chrome after 2 s of idle. Cursor move resets the timer; clicks
    // on chrome surfaces don't count as movement (handled by the body's
    // pointer events). We re-arm whenever the asset id changes.
    useEffect(() => {
        const reset = () => {
            setChromeHidden(false);
            if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current);
            idleTimerRef.current = window.setTimeout(
                () => setChromeHidden(true),
                2000,
            );
        };
        reset();
        window.addEventListener("mousemove", reset);
        return () => {
            if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current);
            window.removeEventListener("mousemove", reset);
        };
    }, [id]);

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
                if (infoOpen) setInfoOpen(false);
                else setView(back);
            } else if (e.key === "f" || e.key === "F") {
                e.preventDefault();
                setShowFaces((v) => !v);
            } else if (e.key === "i" || e.key === "I") {
                e.preventDefault();
                setInfoOpen((v) => !v);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [id, index, neighbors, infoOpen]);

    const { data: detail, isLoading } = useQuery({
        queryKey: ["asset-detail", id],
        queryFn: () => api.assetDetail(id),
        placeholderData: keepPreviousData,
    });

    const { data: albums = [] } = useQuery<AlbumView[]>({
        queryKey: ["albums", hiddenUnlocked ? "withHidden" : "plain"],
        queryFn: () => api.listAlbums(hiddenUnlocked),
    });

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

    const imgRef = useRef<HTMLImageElement | null>(null);
    const [imgDims, setImgDims] = useState<{ w: number; h: number } | null>(null);
    useEffect(() => {
        setImgDims(null);
    }, [id]);

    // Prefetch neighbours.
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

    const [showRaw, setShowRaw] = useState(false);
    const [originalUrl, setOriginalUrl] = useState<string | null>(null);
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
                /* RAW format the browser doesn't decode — leave null */
            }
        })();
        return () => {
            cancelled = true;
            if (url) URL.revokeObjectURL(url);
        };
    }, [id, detail, isPlayable, showRaw]);

    // Date chip uses the actual capture date when available, falling back
    // to the day-bucket. Place chip currently shows "Lat,Lon" because the
    // reverse-geocoded place name lives in the trip layer; Phase 4 wires
    // proper place chips from list_places.
    const dateChip = useMemo(() => {
        if (!detail?.taken_at_utc) return null;
        const d = new Date(detail.taken_at_utc);
        return d.toLocaleDateString(undefined, {
            year: "numeric",
            month: "long",
            day: "numeric",
        });
    }, [detail?.taken_at_utc]);

    const placeChip = useMemo(() => {
        if (!detail?.gps) return null;
        return `${detail.gps.lat.toFixed(2)}, ${detail.gps.lon.toFixed(2)}`;
    }, [detail?.gps]);

    const addToAlbum = async (albumId: number) => {
        await api.addToAlbum(albumId, [id]);
        await queryClient.invalidateQueries({ queryKey: ["albums"] });
    };

    if (isLoading || !detail) {
        return (
            <div className="kp-asset">
                <div className="kp-library-loading">Loading…</div>
            </div>
        );
    }

    return (
        <div className="kp-asset" data-chrome-hidden={chromeHidden ? "true" : undefined}>
            <header className="kp-asset-chrome-top">
                <button
                    type="button"
                    className="kp-asset-back"
                    onClick={() => setView(back)}
                    aria-label="Back"
                >
                    <ArrowLeft size={16} />
                </button>
                <span className="kp-asset-filename">{detail.filename}</span>
                {hasNeighbors && (
                    <span className="kp-asset-counter">
                        {index! + 1} of {neighbors!.length}
                    </span>
                )}
                {!isPlayable && (
                    <button
                        type="button"
                        className="kp-asset-info-button"
                        onClick={() => setShowFaces((v) => !v)}
                        title={showFaces ? "Hide faces (F)" : "Show faces (F)"}
                        aria-label={
                            showFaces ? "Hide face overlays" : "Show face overlays"
                        }
                        data-active={showFaces ? "true" : undefined}
                    >
                        <span style={{ fontSize: 11, fontWeight: 600 }}>F</span>
                    </button>
                )}
                <button
                    type="button"
                    className="kp-asset-info-button"
                    onClick={() => setInfoOpen((v) => !v)}
                    title={infoOpen ? "Close info (I)" : "Open info (I)"}
                    data-active={infoOpen ? "true" : undefined}
                >
                    <Info size={16} />
                </button>
            </header>

            <div className="kp-asset-stage">
                {hasNeighbors && (
                    <>
                        <button
                            type="button"
                            className="kp-asset-nav kp-asset-nav-prev"
                            onClick={goPrev}
                            disabled={prevId === null}
                            aria-label="Previous photo"
                        >
                            <ChevronLeft size={28} />
                        </button>
                        <button
                            type="button"
                            className="kp-asset-nav kp-asset-nav-next"
                            onClick={goNext}
                            disabled={nextId === null}
                            aria-label="Next photo"
                        >
                            <ChevronRight size={28} />
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
                        <div className="kp-library-loading">Loading video…</div>
                    )
                ) : detail.is_raw && showRaw && originalUrl ? (
                    <motion.img
                        layoutId={`asset-${id}`}
                        transition={{
                            duration: 0.32,
                            ease: [0.32, 0.72, 0, 1],
                        }}
                        src={originalUrl}
                        alt={`${detail.filename} (RAW)`}
                    />
                ) : fullUrl ? (
                    <div style={{ position: "relative", maxWidth: "100%", maxHeight: "100%" }}>
                        <motion.img
                            ref={imgRef}
                            layoutId={`asset-${id}`}
                            transition={{
                                duration: 0.32,
                                ease: [0.32, 0.72, 0, 1],
                            }}
                            src={fullUrl}
                            alt={detail.filename}
                            onLoad={(e) => {
                                const el = e.currentTarget;
                                setImgDims({
                                    w: el.naturalWidth,
                                    h: el.naturalHeight,
                                });
                            }}
                            style={{ display: "block", maxWidth: "100%", maxHeight: "100%" }}
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
                    <div className="kp-library-loading" />
                )}
            </div>

            <footer className="kp-asset-chrome-bottom">
                <div className="kp-asset-info-strip">
                    {dateChip && (
                        <EntityChip
                            entity={{
                                kind: "date",
                                utcDay: detail.taken_at_utc_day ?? 0,
                                label: dateChip,
                                precision: "exact",
                            }}
                            onClick={() => setView({ kind: "search" })}
                        />
                    )}
                    {placeChip && (
                        <EntityChip
                            entity={{
                                kind: "place",
                                placeId: `gps:${detail.gps?.lat.toFixed(4)}:${detail.gps?.lon.toFixed(4)}`,
                                name: placeChip,
                            }}
                            onClick={() => setView({ kind: "map" })}
                        />
                    )}
                    {detail.device && (
                        <EntityChip
                            entity={{ kind: "camera", make: detail.device }}
                            onClick={() => setView({ kind: "search" })}
                        />
                    )}
                    {detail.lens && (
                        <EntityChip
                            entity={{ kind: "lens", lens: detail.lens }}
                            onClick={() => setView({ kind: "search" })}
                        />
                    )}
                    {detail.exif_json && (
                        <Popover
                            trigger={
                                <button
                                    type="button"
                                    className="kp-entity-chip"
                                    data-size="sm"
                                >
                                    EXIF
                                </button>
                            }
                            side="top"
                        >
                            <pre
                                style={{
                                    margin: 0,
                                    font: "var(--font-mono)",
                                    fontSize: 11,
                                    maxHeight: 240,
                                    overflow: "auto",
                                }}
                            >
                                {formatExifJson(detail.exif_json)}
                            </pre>
                        </Popover>
                    )}
                </div>
            </footer>

            {infoOpen && (
                <motion.aside
                    className="kp-asset-info-pane"
                    initial={{ x: "100%" }}
                    animate={{ x: 0 }}
                    exit={{ x: "100%" }}
                    transition={{
                        type: "spring",
                        stiffness: 380,
                        damping: 35,
                    }}
                >
                    <div
                        style={{
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "space-between",
                        }}
                    >
                        <h3 style={{ font: "var(--font-title-1)", margin: 0 }}>
                            Photo info
                        </h3>
                        <IconButton
                            icon={<X size={16} />}
                            label="Close info pane"
                            onClick={() => setInfoOpen(false)}
                        />
                    </div>

                    <div className="kp-asset-info-section">
                        <div className="kp-asset-info-row">
                            <span className="label">Type</span>
                            <span className="value">{detail.mime}</span>
                        </div>
                        <div className="kp-asset-info-row">
                            <span className="label">Bytes</span>
                            <span className="value">
                                {detail.bytes.toLocaleString()}
                            </span>
                        </div>
                        {detail.width && detail.height && (
                            <div className="kp-asset-info-row">
                                <span className="label">Dimensions</span>
                                <span className="value">
                                    {detail.width} × {detail.height}
                                </span>
                            </div>
                        )}
                        {detail.duration_ms != null && (
                            <div className="kp-asset-info-row">
                                <span className="label">Duration</span>
                                <span className="value">
                                    {(detail.duration_ms / 1000).toFixed(1)}s
                                </span>
                            </div>
                        )}
                        {detail.taken_at_utc && (
                            <div className="kp-asset-info-row">
                                <span className="label">Taken</span>
                                <span className="value">
                                    {detail.taken_at_utc}
                                </span>
                            </div>
                        )}
                    </div>

                    {(detail.is_live ||
                        detail.is_motion ||
                        detail.is_raw ||
                        detail.is_screenshot ||
                        detail.is_video) && (
                        <div className="kp-asset-info-flags">
                            {detail.is_live && <span className="kp-chip" data-size="sm">Live</span>}
                            {detail.is_motion && <span className="kp-chip" data-size="sm">Motion</span>}
                            {detail.is_raw && <span className="kp-chip" data-size="sm">RAW</span>}
                            {detail.is_screenshot && <span className="kp-chip" data-size="sm">Screenshot</span>}
                            {detail.is_video && <span className="kp-chip" data-size="sm">Video</span>}
                        </div>
                    )}

                    {detail.is_raw && (
                        <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => setShowRaw((s) => !s)}
                            data-testid="raw-toggle"
                        >
                            {showRaw ? "Show JPEG preview" : "Show RAW original"}
                        </Button>
                    )}

                    <div className="kp-asset-info-section">
                        <h4>Add to album</h4>
                        {albums.length === 0 ? (
                            <p
                                style={{
                                    margin: 0,
                                    color: "var(--color-text-tertiary)",
                                    font: "var(--font-caption)",
                                }}
                            >
                                No albums yet.
                            </p>
                        ) : (
                            <div className="kp-asset-info-album-list">
                                {albums.map((a) => (
                                    <Button
                                        key={a.id}
                                        size="sm"
                                        variant="ghost"
                                        onClick={() => addToAlbum(a.id)}
                                        style={{ justifyContent: "flex-start" }}
                                    >
                                        + {a.name}
                                    </Button>
                                ))}
                            </div>
                        )}
                    </div>

                    <CaptionEditor assetId={id} />
                    <PetFlagEditor assetId={id} />
                </motion.aside>
            )}
        </div>
    );
}

function formatExifJson(raw: string): string {
    try {
        return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
        return raw;
    }
}

function CaptionEditor({ assetId }: { assetId: number }) {
    const [text, setText] = useState("");
    const [busy, setBusy] = useState(false);

    const save = async () => {
        setBusy(true);
        try {
            await api.indexAssetText(assetId, text);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="kp-asset-info-section">
            <h4>Caption</h4>
            <p
                style={{
                    margin: 0,
                    color: "var(--color-text-tertiary)",
                    font: "var(--font-caption)",
                }}
            >
                Searchable. Tokens are HMAC'd under your search key.
            </p>
            <textarea
                rows={3}
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Caption, tags, anything you want to find this by…"
                disabled={busy}
            />
            <Button size="sm" variant="primary" onClick={save} loading={busy}>
                Save caption
            </Button>
        </div>
    );
}

function PetFlagEditor({ assetId }: { assetId: number }) {
    const queryClient = useQueryClient();
    const [species, setSpecies] = useState("");
    const [busy, setBusy] = useState(false);

    const mark = async (isPet: boolean) => {
        setBusy(true);
        try {
            await api.setAssetPet(assetId, isPet, isPet ? species.trim() || null : null);
            await queryClient.invalidateQueries({ queryKey: ["pets"] });
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="kp-asset-info-section">
            <h4>Pet</h4>
            <input
                type="text"
                placeholder="Species (dog, cat, bird, …)"
                value={species}
                onChange={(e) => setSpecies(e.target.value)}
                disabled={busy}
                style={{
                    background: "var(--color-surface-2)",
                    border: "1px solid var(--color-border-default)",
                    borderRadius: "var(--radius-md)",
                    color: "var(--color-text-primary)",
                    padding: "var(--space-2) var(--space-3)",
                    font: "var(--font-body)",
                }}
            />
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
                <Button size="sm" variant="primary" onClick={() => mark(true)} loading={busy}>
                    Mark as pet
                </Button>
                <Button size="sm" variant="ghost" onClick={() => mark(false)} loading={busy}>
                    Not a pet
                </Button>
            </div>
        </div>
    );
}
