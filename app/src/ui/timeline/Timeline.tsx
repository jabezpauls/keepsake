import { useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { TimelineCursor } from "../../bindings/TimelineCursor";
import type { TimelineEntryView } from "../../bindings/TimelineEntryView";
import ThumbImage from "./ThumbImage";

type Zoom = "day" | "month" | "year";

const ZOOM_COLS: Record<Zoom, number> = { day: 4, month: 6, year: 10 };
const ZOOM_CELL: Record<Zoom, number> = { day: 220, month: 160, year: 100 };

export default function Timeline() {
    return (
        <CollectionView
            fetchPage={(cursor) => api.timelinePage(cursor, 120)}
            queryKey={["timeline"]}
        />
    );
}

interface Props {
    queryKey: readonly unknown[];
    fetchPage: (cursor: TimelineCursor | null) => Promise<{
        entries: TimelineEntryView[];
        next_cursor: TimelineCursor | null;
    }>;
}

// Flat item list used by the virtualizer. "header" rows render sticky month
// titles; "photos" rows render N thumbnails.
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

export function CollectionView({ queryKey, fetchPage }: Props) {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const [zoom, setZoom] = useState<Zoom>("month");
    const [cursorIdx, setCursorIdx] = useState(0);

    const query = useInfiniteQuery({
        queryKey: [...queryKey, zoom],
        initialPageParam: null as TimelineCursor | null,
        queryFn: async ({ pageParam }) => fetchPage(pageParam),
        getNextPageParam: (last) => last.next_cursor,
    });

    const allEntries: TimelineEntryView[] = useMemo(
        () => query.data?.pages.flatMap((p) => p.entries) ?? [],
        [query.data],
    );

    const cols = ZOOM_COLS[zoom];
    const cell = ZOOM_CELL[zoom];

    // Build rows: interleave a header row whenever the group label changes,
    // then pack entries `cols`-at-a-time under it.
    const rows: Row[] = useMemo(() => {
        const out: Row[] = [];
        if (allEntries.length === 0) return out;
        let currentLabel = "";
        let group: TimelineEntryView[] = [];
        const flushGroup = () => {
            for (let i = 0; i < group.length; i += cols) {
                out.push({ kind: "photos", entries: group.slice(i, i + cols) });
            }
            group = [];
        };
        for (const e of allEntries) {
            const label = dayToLabel(e.taken_at_utc_day, zoom);
            if (label !== currentLabel) {
                flushGroup();
                out.push({ kind: "header", label });
                currentLabel = label;
            }
            group.push(e);
        }
        flushGroup();
        return out;
    }, [allEntries, cols, zoom]);

    const scrollRef = useRef<HTMLDivElement | null>(null);
    const rowVirtualizer = useVirtualizer({
        count: rows.length,
        getScrollElement: () => scrollRef.current,
        estimateSize: (i) => (rows[i]?.kind === "header" ? 36 : cell),
        overscan: 4,
    });

    // Infinite scroll sentinel.
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

    // Keyboard navigation. j/k: page, arrows: move cursor over photo cells.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
                return;
            }
            const photoIndex: number[] = [];
            rows.forEach((r, i) => r.kind === "photos" && photoIndex.push(i));
            if (!photoIndex.length) return;
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
                setCursorIdx((i) => Math.min(allEntries.length - 1, i + cols));
            } else if (e.key === "ArrowUp") {
                setCursorIdx((i) => Math.max(0, i - cols));
            } else if (e.key === "Enter") {
                const entry = allEntries[cursorIdx];
                if (entry) {
                    setView({ kind: "asset", id: entry.id, back: currentView });
                }
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [rows, cols, allEntries, cursorIdx, rowVirtualizer, setView, currentView]);

    if (query.isLoading) return <div className="timeline-loading">Loading…</div>;
    if (query.isError) return <div className="timeline-error">Failed to load timeline.</div>;
    if (allEntries.length === 0) {
        return (
            <div className="timeline-empty">
                <p>No assets yet.</p>
                <p>Add a source to get started.</p>
            </div>
        );
    }

    return (
        <div className="timeline-container">
            <div className="timeline-zoom">
                {(["year", "month", "day"] as const).map((z) => (
                    <button
                        key={z}
                        className={z === zoom ? "active" : ""}
                        onClick={() => setZoom(z)}
                    >
                        {z}
                    </button>
                ))}
            </div>
            <div ref={scrollRef} className="timeline-scroller">
                <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative" }}>
                    {virtualRows.map((row) => {
                        const rowData = rows[row.index];
                        if (!rowData) return null;
                        if (rowData.kind === "header") {
                            return (
                                <div
                                    key={row.index}
                                    className="timeline-header"
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
                                className="timeline-row"
                                style={{
                                    position: "absolute",
                                    top: 0,
                                    transform: `translateY(${row.start}px)`,
                                    height: `${cell}px`,
                                    width: "100%",
                                    gridTemplateColumns: `repeat(${cols}, 1fr)`,
                                }}
                            >
                                {rowData.entries.map((e) => {
                                    const idx = allEntries.indexOf(e);
                                    return (
                                        <button
                                            key={e.id}
                                            className={`timeline-cell${idx === cursorIdx ? " cursor" : ""}`}
                                            onClick={() => {
                                                setCursorIdx(idx);
                                                setView({
                                                    kind: "asset",
                                                    id: e.id,
                                                    back: currentView,
                                                });
                                            }}
                                        >
                                            <ThumbImage
                                                assetId={e.id}
                                                size={zoom === "year" ? 256 : 256}
                                                mime={e.mime}
                                                alt=""
                                            />
                                            {e.is_video && <span className="cell-badge">▶</span>}
                                            {e.is_live && <span className="cell-badge">LIVE</span>}
                                            {e.is_raw && <span className="cell-badge raw">RAW</span>}
                                        </button>
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
