import { useInfiniteQuery } from "@tanstack/react-query";
import { useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { TimelineCursor } from "../../bindings/TimelineCursor";
import type { TimelineEntryView } from "../../bindings/TimelineEntryView";
import ThumbImage from "./ThumbImage";

const COLS = 5;
const CELL = 180;

export default function Timeline() {
    return <CollectionView fetchPage={(cursor) => api.timelinePage(cursor, 120)} queryKey={["timeline"]} />;
}

interface Props {
    queryKey: readonly unknown[];
    fetchPage: (cursor: TimelineCursor | null) => Promise<{
        entries: TimelineEntryView[];
        next_cursor: TimelineCursor | null;
    }>;
}

export function CollectionView({ queryKey, fetchPage }: Props) {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const query = useInfiniteQuery({
        queryKey,
        initialPageParam: null as TimelineCursor | null,
        queryFn: async ({ pageParam }) => fetchPage(pageParam),
        getNextPageParam: (last) => last.next_cursor,
    });

    const allEntries: TimelineEntryView[] = useMemo(() => {
        return query.data?.pages.flatMap((p) => p.entries) ?? [];
    }, [query.data]);

    const rowCount = Math.ceil(allEntries.length / COLS);
    const scrollRef = useRef<HTMLDivElement | null>(null);
    const rowVirtualizer = useVirtualizer({
        count: rowCount,
        getScrollElement: () => scrollRef.current,
        estimateSize: () => CELL,
        overscan: 2,
    });

    // Near-bottom sentinel: when user scrolls past the last rendered row and
    // another page exists, fetch it.
    const virtualRows = rowVirtualizer.getVirtualItems();
    const lastRow = virtualRows[virtualRows.length - 1];
    if (
        lastRow &&
        lastRow.index >= rowCount - 2 &&
        query.hasNextPage &&
        !query.isFetchingNextPage
    ) {
        void query.fetchNextPage();
    }

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
        <div ref={scrollRef} className="timeline-scroller">
            <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative" }}>
                {virtualRows.map((row) => {
                    const start = row.index * COLS;
                    const rowEntries = allEntries.slice(start, start + COLS);
                    return (
                        <div
                            key={row.index}
                            className="timeline-row"
                            style={{
                                position: "absolute",
                                top: 0,
                                transform: `translateY(${row.start}px)`,
                                height: `${CELL}px`,
                                width: "100%",
                            }}
                        >
                            {rowEntries.map((e) => (
                                <button
                                    key={e.id}
                                    className="timeline-cell"
                                    onClick={() => setView({ kind: "asset", id: e.id, back: currentView })}
                                >
                                    <ThumbImage
                                        assetId={e.id}
                                        size={256}
                                        mime={e.mime}
                                        alt=""
                                    />
                                    {e.is_video && <span className="cell-badge">▶</span>}
                                    {e.is_live && <span className="cell-badge">LIVE</span>}
                                </button>
                            ))}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
