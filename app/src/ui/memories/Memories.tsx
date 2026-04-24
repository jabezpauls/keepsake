import { useQuery } from "@tanstack/react-query";
import { api, bytesToBlobUrl } from "../../ipc";
import type { MemoryGroupView } from "../../bindings/MemoryGroupView";
import { useSession } from "../../state/session";

export default function Memories() {
    const setView = useSession((s) => s.setView);
    const groups = useQuery<MemoryGroupView[]>({
        queryKey: ["memories", "on-this-day"],
        queryFn: api.memoriesOnThisDay,
    });

    const today = new Date();
    const todayLabel = today.toLocaleDateString(undefined, {
        month: "long",
        day: "numeric",
    });

    return (
        <div className="memories-view">
            <nav className="memories-nav">
                <h2>Memories · {todayLabel}</h2>
            </nav>
            {groups.isLoading && <p>Loading…</p>}
            {groups.data && groups.data.length === 0 && (
                <p className="muted">
                    Nothing from this day in past years — come back later.
                </p>
            )}
            {(groups.data ?? []).map((g) => (
                <section key={g.year} className="memory-group">
                    <h3>
                        {g.years_ago === 1
                            ? "1 year ago"
                            : `${g.years_ago} years ago`}{" "}
                        · <span className="muted">{g.year}</span>{" "}
                        <span className="count">
                            ({g.asset_ids.length})
                        </span>
                    </h3>
                    <div className="memory-strip">
                        {g.asset_ids.slice(0, 8).map((id: number, idx: number) => (
                            <MemoryThumb
                                key={id}
                                assetId={id}
                                onOpen={() =>
                                    setView({
                                        kind: "asset",
                                        id,
                                        back: { kind: "memories" },
                                        neighbors: g.asset_ids,
                                        index: idx,
                                    })
                                }
                            />
                        ))}
                        {g.asset_ids.length > 8 && (
                            <div className="memory-overflow muted">
                                +{g.asset_ids.length - 8}
                            </div>
                        )}
                    </div>
                </section>
            ))}
        </div>
    );
}

function MemoryThumb({
    assetId,
    onOpen,
}: {
    assetId: number;
    onOpen: () => void;
}) {
    const thumb = useQuery({
        queryKey: ["memory-thumb", assetId],
        queryFn: () => api.assetThumbnail(assetId, 256),
        staleTime: 10 * 60_000,
    });
    const url = thumb.data ? bytesToBlobUrl(thumb.data, "image/webp") : null;
    return (
        <button
            className="memory-thumb"
            onClick={onOpen}
            aria-label={`Open photo ${assetId}`}
        >
            {url ? (
                <img src={url} alt="" />
            ) : (
                <div className="memory-thumb-placeholder" />
            )}
        </button>
    );
}
