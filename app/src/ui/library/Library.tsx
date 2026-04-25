import { useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { motion } from "framer-motion";
import { Image as ImageIcon, Plus } from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { TimelineEntryView } from "../../bindings/TimelineEntryView";
import ThumbImage from "../timeline/ThumbImage";
import { Button, Chip, EmptyState } from "../../components";
import "./library.css";

type Zoom = "day" | "month" | "year";

interface ZoomConfig {
    cols: number;
    cell: number;
    gap: number;
    label: string;
}

const ZOOM: Record<Zoom, ZoomConfig> = {
    day: { cols: 4, cell: 240, gap: 6, label: "Day" },
    month: { cols: 6, cell: 180, gap: 4, label: "Month" },
    year: { cols: 10, cell: 110, gap: 2, label: "Year" },
};

type Row =
    | { kind: "header"; label: string }
    | { kind: "photos"; entries: TimelineEntryView[] };

function dayToLabel(day: number | null, zoom: Zoom): string {
    if (day === null) return "Undated";
    const d = new Date(day * 86_400_000);
    if (zoom === "year") return d.toLocaleDateString(undefined, { year: "numeric" });
    if (zoom === "month")
        return d.toLocaleDateString(undefined, { year: "numeric", month: "long" });
    return d.toLocaleDateString(undefined, {
        year: "numeric",
        month: "long",
        day: "numeric",
    });
}

// Phase 3 Library — replaces the legacy Timeline as the hero surface.
//
// What's new vs. Timeline:
//   - Edge-to-edge cells with token-driven gaps (4 px → 2 px → 0 as the
//     user zooms out, matching Apple's tighter mosaic for years).
//   - Cells wear `motion.div` with `layoutId={`asset-${id}`}` so opening
//     the photo fires a shared-element transition (matched in
//     AssetDetail with the same layoutId).
//   - Sticky chrome on top fades on scroll — but the legacy Timeline has
//     it as inline buttons; we keep that pattern for Phase 3 and let
//     Phase 9 polish the auto-fade.
//   - Empty + loading states use the EmptyState / Skeleton primitives
//     from Phase 1.
//
// Backwards-compat: keyboard shortcuts (j/k/arrows/Enter) preserved.
// Asset opens still set `back: currentView` on the asset view so legacy
// AssetDetail keeps working — Phase 3's new AssetDetail picks up the
// same `back` field via the View union.
export default function Library() {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const [zoom, setZoom] = useState<Zoom>("month");
    const [cursorIdx, setCursorIdx] = useState(0);

    const query = useInfiniteQuery({
        queryKey: ["timeline", zoom],
        initialPageParam: null as Parameters<typeof api.timelinePage>[0],
        queryFn: async ({ pageParam }) => api.timelinePage(pageParam, 120),
        getNextPageParam: (last) => last.next_cursor,
    });

    const allEntries: TimelineEntryView[] = useMemo(
        () => query.data?.pages.flatMap((p) => p.entries) ?? [],
        [query.data],
    );

    const cfg = ZOOM[zoom];

    const rows: Row[] = useMemo(() => {
        const out: Row[] = [];
        if (allEntries.length === 0) return out;
        let currentLabel = "";
        let group: TimelineEntryView[] = [];
        const flush = () => {
            for (let i = 0; i < group.length; i += cfg.cols) {
                out.push({ kind: "photos", entries: group.slice(i, i + cfg.cols) });
            }
            group = [];
        };
        for (const e of allEntries) {
            const label = dayToLabel(e.taken_at_utc_day, zoom);
            if (label !== currentLabel) {
                flush();
                out.push({ kind: "header", label });
                currentLabel = label;
            }
            group.push(e);
        }
        flush();
        return out;
    }, [allEntries, cfg.cols, zoom]);

    const scrollRef = useRef<HTMLDivElement | null>(null);
    const rowVirtualizer = useVirtualizer({
        count: rows.length,
        getScrollElement: () => scrollRef.current,
        estimateSize: (i) =>
            rows[i]?.kind === "header" ? 40 : cfg.cell + cfg.gap,
        overscan: 4,
    });

    const virtualRows = rowVirtualizer.getVirtualItems();
    const lastRow = virtualRows[virtualRows.length - 1];
    if (
        lastRow &&
        lastRow.index >= rows.length - 3 &&
        query.hasNextPage &&
        !query.isFetchingNextPage
    ) {
        void query.fetchNextPage();
    }

    const openAsset = (entry: TimelineEntryView, idx: number) => {
        setCursorIdx(idx);
        setView({
            kind: "asset",
            id: entry.id,
            back: currentView,
            neighbors: allEntries.map((a) => a.id),
            index: idx,
        });
    };

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (
                e.target instanceof HTMLInputElement ||
                e.target instanceof HTMLTextAreaElement
            ) {
                return;
            }
            if (e.key === "j" || e.key === "PageDown") {
                e.preventDefault();
                rowVirtualizer.scrollBy(500);
            } else if (e.key === "k" || e.key === "PageUp") {
                e.preventDefault();
                rowVirtualizer.scrollBy(-500);
            } else if (e.key === "ArrowRight") {
                setCursorIdx((i) => Math.min(allEntries.length - 1, i + 1));
            } else if (e.key === "ArrowLeft") {
                setCursorIdx((i) => Math.max(0, i - 1));
            } else if (e.key === "ArrowDown") {
                setCursorIdx((i) => Math.min(allEntries.length - 1, i + cfg.cols));
            } else if (e.key === "ArrowUp") {
                setCursorIdx((i) => Math.max(0, i - cfg.cols));
            } else if (e.key === "Enter") {
                const entry = allEntries[cursorIdx];
                if (entry) openAsset(entry, cursorIdx);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [rows, allEntries, cursorIdx, cfg.cols]);

    if (query.isLoading) {
        return (
            <div className="kp-library">
                <div className="kp-library-loading">Loading…</div>
            </div>
        );
    }
    if (query.isError) {
        return (
            <div className="kp-library">
                <EmptyState
                    title="Couldn't load the library"
                    hint="Open Settings → ML to verify the runtime, or reload."
                />
            </div>
        );
    }
    if (allEntries.length === 0) {
        return (
            <div className="kp-library">
                <EmptyState
                    icon={<ImageIcon size={36} />}
                    title="Your library is empty"
                    hint="Add a source to start importing photos and videos."
                    actions={
                        <Button
                            variant="primary"
                            leadingIcon={<Plus size={14} />}
                            onClick={() => setView({ kind: "sources" })}
                        >
                            Add source
                        </Button>
                    }
                />
            </div>
        );
    }

    return (
        <div className="kp-library">
            <header className="kp-library-toolbar">
                <h1 className="kp-library-title">Library</h1>
                <div className="kp-library-zoom" role="group" aria-label="Zoom">
                    {(Object.keys(ZOOM) as Zoom[]).map((z) => (
                        <Chip
                            key={z}
                            active={z === zoom}
                            onClick={() => setZoom(z)}
                        >
                            {ZOOM[z].label}
                        </Chip>
                    ))}
                </div>
            </header>
            <div ref={scrollRef} className="kp-library-scroller">
                <div
                    style={{
                        height: rowVirtualizer.getTotalSize(),
                        position: "relative",
                    }}
                >
                    {virtualRows.map((row) => {
                        const rowData = rows[row.index];
                        if (!rowData) return null;
                        if (rowData.kind === "header") {
                            return (
                                <div
                                    key={row.index}
                                    className="kp-library-header"
                                    style={{
                                        position: "absolute",
                                        top: 0,
                                        transform: `translateY(${row.start}px)`,
                                        width: "100%",
                                        height: row.size,
                                    }}
                                >
                                    {rowData.label}
                                </div>
                            );
                        }
                        return (
                            <div
                                key={row.index}
                                className="kp-library-row"
                                data-zoom={zoom}
                                style={{
                                    position: "absolute",
                                    top: 0,
                                    transform: `translateY(${row.start}px)`,
                                    height: `${cfg.cell}px`,
                                    width: "100%",
                                    gridTemplateColumns: `repeat(${cfg.cols}, 1fr)`,
                                    gap: `${cfg.gap}px`,
                                }}
                            >
                                {rowData.entries.map((e) => {
                                    const idx = allEntries.indexOf(e);
                                    return (
                                        <LibraryCell
                                            key={e.id}
                                            entry={e}
                                            zoom={zoom}
                                            isCursor={idx === cursorIdx}
                                            onClick={() => openAsset(e, idx)}
                                        />
                                    );
                                })}
                            </div>
                        );
                    })}
                </div>
            </div>
        </div>
    );
}

interface LibraryCellProps {
    entry: TimelineEntryView;
    zoom: Zoom;
    isCursor: boolean;
    onClick: () => void;
}

// One cell. Wrapped in `motion.div` with `layoutId={`asset-${id}`}` so the
// thumbnail seamlessly expands into AssetDetail's full-bleed photo when
// the user clicks. The matching motion element on the other side carries
// the same layoutId — framer-motion bridges the transition automatically.
function LibraryCell({ entry, zoom, isCursor, onClick }: LibraryCellProps) {
    return (
        <motion.button
            type="button"
            layoutId={`asset-${entry.id}`}
            transition={{
                duration: 0.32,
                ease: [0.32, 0.72, 0, 1], // matches --ease-spring
            }}
            className="kp-library-cell"
            data-cursor={isCursor ? "true" : undefined}
            onClick={onClick}
        >
            <ThumbImage
                assetId={entry.id}
                size={zoom === "year" ? 256 : 256}
                mime={entry.mime}
                alt=""
            />
            {(entry.is_video || entry.is_live || entry.is_raw) && (
                <div className="kp-library-cell-badges">
                    {entry.is_video && <span className="kp-library-cell-badge">▶</span>}
                    {entry.is_live && (
                        <span className="kp-library-cell-badge">LIVE</span>
                    )}
                    {entry.is_raw && (
                        <span className="kp-library-cell-badge kp-library-cell-badge-raw">
                            RAW
                        </span>
                    )}
                </div>
            )}
        </motion.button>
    );
}
