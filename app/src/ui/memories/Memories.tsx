import { useQuery } from "@tanstack/react-query";
import { api, bytesToBlobUrl } from "../../ipc";
import type { MemoryGroupView } from "../../bindings/MemoryGroupView";
import type { YearInPhotosView } from "../../bindings/YearInPhotosView";
import type { PersonYearMemoryView } from "../../bindings/PersonYearMemoryView";
import type { PersonView } from "../../bindings/PersonView";
import { useSession } from "../../state/session";

export default function Memories() {
    const setView = useSession((s) => s.setView);
    const groups = useQuery<MemoryGroupView[]>({
        queryKey: ["memories", "on-this-day"],
        queryFn: api.memoriesOnThisDay,
    });
    const years = useQuery<YearInPhotosView[]>({
        queryKey: ["memories", "year-in-photos"],
        queryFn: api.memoriesYearInPhotos,
    });
    const personYears = useQuery<PersonYearMemoryView[]>({
        queryKey: ["memories", "person-year"],
        queryFn: () => api.memoriesPersonYear(3),
    });
    const people = useQuery<PersonView[]>({
        queryKey: ["people"],
        queryFn: () => api.listPeople(false),
    });
    const personNameById = new Map<number, string>();
    for (const p of people.data ?? []) {
        if (p.name) personNameById.set(p.id, p.name);
    }

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

            {/* On this day */}
            <h3 className="memories-section-h">On this day</h3>
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
                        <span className="count">({g.asset_ids.length})</span>
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

            {/* Year in photos */}
            {(years.data ?? []).length > 0 && (
                <>
                    <h3 className="memories-section-h">Year in photos</h3>
                    {(years.data ?? []).map((y) => (
                        <section key={`year-${y.year}`} className="memory-group">
                            <h3>
                                {y.year} ·{" "}
                                <span className="count">
                                    {y.asset_count} photos
                                </span>
                            </h3>
                            <div className="memory-strip">
                                {y.highlights.map((id, idx) => (
                                    <MemoryThumb
                                        key={id}
                                        assetId={id}
                                        onOpen={() =>
                                            setView({
                                                kind: "asset",
                                                id,
                                                back: { kind: "memories" },
                                                neighbors: y.highlights,
                                                index: idx,
                                            })
                                        }
                                    />
                                ))}
                            </div>
                        </section>
                    ))}
                </>
            )}

            {/* Person × Year */}
            {(personYears.data ?? []).length > 0 && (
                <>
                    <h3 className="memories-section-h">People across the years</h3>
                    {(personYears.data ?? []).map((pm) => {
                        const name = personNameById.get(pm.person_id) ?? `Person #${pm.person_id}`;
                        return (
                            <section
                                key={`py-${pm.person_id}-${pm.year}`}
                                className="memory-group"
                            >
                                <h3>
                                    {name} in {pm.year}{" "}
                                    <span className="count">({pm.asset_ids.length})</span>
                                </h3>
                                <div className="memory-strip">
                                    {pm.asset_ids
                                        .slice(0, 8)
                                        .map((id, idx) => (
                                            <MemoryThumb
                                                key={id}
                                                assetId={id}
                                                onOpen={() =>
                                                    setView({
                                                        kind: "asset",
                                                        id,
                                                        back: { kind: "memories" },
                                                        neighbors: pm.asset_ids,
                                                        index: idx,
                                                    })
                                                }
                                            />
                                        ))}
                                    {pm.asset_ids.length > 8 && (
                                        <div className="memory-overflow muted">
                                            +{pm.asset_ids.length - 8}
                                        </div>
                                    )}
                                </div>
                            </section>
                        );
                    })}
                </>
            )}
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
