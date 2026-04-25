import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { motion } from "framer-motion";
import {
    Aperture,
    Camera,
    Image as ImageIcon,
    Search as SearchIcon,
    SlidersHorizontal,
} from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { PersonView } from "../../bindings/PersonView";
import type { PlaceView } from "../../bindings/PlaceView";
import type { TripView } from "../../bindings/TripView";
import type { AlbumView } from "../../bindings/AlbumView";
import type { TimelineEntryView } from "../../bindings/TimelineEntryView";
import type { SearchRequest } from "../../bindings/SearchRequest";
import {
    Avatar,
    Button,
    Card,
    Chip,
    EntityChip,
    Sheet,
} from "../../components";
import ThumbImage from "../timeline/ThumbImage";
import "./search_hub.css";

// Phase 7 SearchHub — Google-Photos-style search-as-discovery surface.
//
// Default state (no query): shows People avatar carousel, Places card
// grid, Categories pill row, Recent imports thumbnail row. The user
// can browse the library via these carousels without typing anything.
//
// Type a query → carousels collapse and we fan out to:
//   * listPeople()    — name-prefix match → People hits
//   * listPlaces()    — city-prefix match → Place hits
//   * listTrips()     — trip-name match → Trip hits
//   * listAlbums()    — album-name match → Album hits
//   * searchAssets()  — CLIP/text-search → Photo grid
//
// Non-photo hits render as EntityChips at the top, photos as a grid
// below. The existing chip filters (RAW / Video / Screenshot / Live /
// faces / camera / lens / date) move into a "Filters" Sheet behind a
// SlidersHorizontal button.
export default function SearchHub() {
    const setView = useSession((s) => s.setView);
    const pushView = useSession((s) => s.pushView);
    const currentView = useSession((s) => s.view);

    const [query, setQuery] = useState("");
    const [filtersOpen, setFiltersOpen] = useState(false);
    const inputRef = useRef<HTMLInputElement | null>(null);

    type ToggleKey = "is_raw" | "is_video" | "is_screenshot" | "is_live" | "has_faces";
    const [toggles, setToggles] = useState<Record<ToggleKey, boolean | null>>({
        is_raw: null,
        is_video: null,
        is_screenshot: null,
        is_live: null,
        has_faces: null,
    });
    const [camera, setCamera] = useState("");
    const [lens, setLens] = useState("");

    useEffect(() => {
        inputRef.current?.focus();
    }, []);

    // Pre-load discovery sources unconditionally — they double as
    // typed-search resolvers, so caching them up front keeps results
    // instant once the user starts typing.
    const people = useQuery<PersonView[]>({
        queryKey: ["people"],
        queryFn: () => api.listPeople(false),
    });
    const places = useQuery<PlaceView[]>({
        queryKey: ["places"],
        queryFn: () => api.listPlaces(),
    });
    const trips = useQuery<TripView[]>({
        queryKey: ["trips"],
        queryFn: api.listTrips,
    });
    const albums = useQuery<AlbumView[]>({
        queryKey: ["albums", "plain"],
        queryFn: () => api.listAlbums(false),
    });
    const recent = useQuery<TimelineEntryView[]>({
        queryKey: ["timeline", "recent-imports"],
        queryFn: async () => (await api.timelinePage(null, 30)).entries,
    });

    // Build the asset-search request when the user has typed text or
    // toggled a filter. Empty queries skip the fetch (the searchAssets
    // command requires a non-empty payload).
    const searchRequest = useMemo<SearchRequest | null>(() => {
        const hasFilter =
            !!query.trim() ||
            !!camera.trim() ||
            !!lens.trim() ||
            Object.values(toggles).some((v) => v != null);
        if (!hasFilter) return null;
        return {
            text: query.trim() || null,
            person_ids: [],
            after_day: null,
            before_day: null,
            source_id: null,
            has_faces: toggles.has_faces,
            is_video: toggles.is_video,
            is_raw: toggles.is_raw,
            is_screenshot: toggles.is_screenshot,
            is_live: toggles.is_live,
            camera_make: camera.trim() || null,
            lens: lens.trim() || null,
            limit: 200,
        };
    }, [query, camera, lens, toggles]);

    const photoHits = useQuery({
        queryKey: ["search", searchRequest],
        queryFn: () =>
            searchRequest
                ? api.searchAssets(searchRequest)
                : Promise.resolve([]),
        enabled: !!searchRequest,
    });

    // Fan out the typed query against entity caches. Phase 7 keeps this
    // simple — case-insensitive contains. Phase 9 polish can swap to a
    // proper fuzzy ranker if needed.
    const q = query.trim().toLowerCase();
    const personHits =
        q.length === 0
            ? []
            : (people.data ?? [])
                  .filter((p) => p.name?.toLowerCase().includes(q))
                  .slice(0, 5);
    const placeHits =
        q.length === 0
            ? []
            : (places.data ?? [])
                  .filter(
                      (p) =>
                          p.city.toLowerCase().includes(q) ||
                          p.country.toLowerCase().includes(q),
                  )
                  .slice(0, 5);
    const tripHits =
        q.length === 0
            ? []
            : (trips.data ?? [])
                  .filter((t) => t.name.toLowerCase().includes(q))
                  .slice(0, 5);
    const albumHits =
        q.length === 0
            ? []
            : (albums.data ?? [])
                  .filter((a) => a.name.toLowerCase().includes(q))
                  .slice(0, 5);

    const isTyping = q.length > 0 || searchRequest != null;

    return (
        <div className="kp-search-hub">
            <header className="kp-search-hub-header">
                <h1>Search</h1>
                <div className="kp-search-hub-input-row">
                    <div className="kp-search-hub-input-wrap">
                        <SearchIcon
                            size={16}
                            className="kp-search-hub-input-icon"
                            aria-hidden
                        />
                        <input
                            ref={inputRef}
                            type="search"
                            className="kp-search-hub-input"
                            placeholder="People, places, things, dates…"
                            value={query}
                            onChange={(e) => setQuery(e.target.value)}
                        />
                    </div>
                    <Button
                        variant="secondary"
                        leadingIcon={<SlidersHorizontal size={14} />}
                        onClick={() => setFiltersOpen(true)}
                    >
                        Filters
                    </Button>
                </div>
            </header>

            {!isTyping ? (
                <DiscoverySurface
                    people={people.data ?? []}
                    places={places.data ?? []}
                    recent={recent.data ?? []}
                    onSelectPerson={(p) =>
                        pushView({ kind: "person", id: p.id, name: p.name })
                    }
                    onSelectPlace={(p) =>
                        pushView({
                            kind: "place",
                            placeId: p.place_id,
                            name: `${p.city}, ${p.country}`,
                        })
                    }
                    onSelectAsset={(id, neighbors, idx) =>
                        pushView({
                            kind: "asset",
                            id,
                            back: currentView,
                            neighbors,
                            index: idx,
                        })
                    }
                    onCategoryClick={(category) => {
                        if (category === "is_raw") setToggles((t) => ({ ...t, is_raw: true }));
                        if (category === "is_video") setToggles((t) => ({ ...t, is_video: true }));
                        if (category === "is_screenshot")
                            setToggles((t) => ({ ...t, is_screenshot: true }));
                        if (category === "is_live") setToggles((t) => ({ ...t, is_live: true }));
                    }}
                    onAllPeople={() => setView({ kind: "people" })}
                    onAllPlaces={() => setView({ kind: "places" })}
                    onAllRecent={() => setView({ kind: "library" })}
                />
            ) : (
                <SearchResults
                    query={q}
                    personHits={personHits}
                    placeHits={placeHits}
                    tripHits={tripHits}
                    albumHits={albumHits}
                    photoHits={photoHits.data ?? []}
                    isLoading={photoHits.isLoading}
                    onSelectPerson={(p) =>
                        pushView({ kind: "person", id: p.id, name: p.name })
                    }
                    onSelectPlace={(p) =>
                        pushView({
                            kind: "place",
                            placeId: p.place_id,
                            name: `${p.city}, ${p.country}`,
                        })
                    }
                    onSelectTrip={(t) =>
                        pushView({
                            kind: "album",
                            id: t.id,
                            name: t.name,
                            source: "trip",
                        })
                    }
                    onSelectAlbum={(a) =>
                        pushView({ kind: "album", id: a.id, name: a.name })
                    }
                    onSelectAsset={(id, neighbors, idx) =>
                        pushView({
                            kind: "asset",
                            id,
                            back: currentView,
                            neighbors,
                            index: idx,
                        })
                    }
                />
            )}

            <Sheet
                open={filtersOpen}
                onOpenChange={setFiltersOpen}
                title="Search filters"
                description="Combine with the typed query — applies to photos, not entity matches."
            >
                <div className="kp-stack">
                    <FilterToggleRow
                        label="Has faces"
                        value={toggles.has_faces}
                        onChange={(v) =>
                            setToggles((t) => ({ ...t, has_faces: v }))
                        }
                    />
                    <FilterToggleRow
                        label="Video"
                        value={toggles.is_video}
                        onChange={(v) => setToggles((t) => ({ ...t, is_video: v }))}
                    />
                    <FilterToggleRow
                        label="RAW"
                        value={toggles.is_raw}
                        onChange={(v) => setToggles((t) => ({ ...t, is_raw: v }))}
                    />
                    <FilterToggleRow
                        label="Screenshot"
                        value={toggles.is_screenshot}
                        onChange={(v) =>
                            setToggles((t) => ({ ...t, is_screenshot: v }))
                        }
                    />
                    <FilterToggleRow
                        label="Live photo"
                        value={toggles.is_live}
                        onChange={(v) => setToggles((t) => ({ ...t, is_live: v }))}
                    />

                    <label className="kp-search-hub-filter-input">
                        <span>
                            <Camera size={14} />
                            Camera make
                        </span>
                        <input
                            type="text"
                            placeholder="e.g. SONY, Apple, Canon"
                            value={camera}
                            onChange={(e) => setCamera(e.target.value)}
                        />
                    </label>
                    <label className="kp-search-hub-filter-input">
                        <span>
                            <Aperture size={14} />
                            Lens
                        </span>
                        <input
                            type="text"
                            placeholder="e.g. FE 24-70mm"
                            value={lens}
                            onChange={(e) => setLens(e.target.value)}
                        />
                    </label>

                    <Button
                        variant="ghost"
                        onClick={() => {
                            setToggles({
                                is_raw: null,
                                is_video: null,
                                is_screenshot: null,
                                is_live: null,
                                has_faces: null,
                            });
                            setCamera("");
                            setLens("");
                        }}
                    >
                        Clear filters
                    </Button>
                </div>
            </Sheet>
        </div>
    );
}

interface DiscoveryProps {
    people: PersonView[];
    places: PlaceView[];
    recent: TimelineEntryView[];
    onSelectPerson: (p: PersonView) => void;
    onSelectPlace: (p: PlaceView) => void;
    onSelectAsset: (id: number, neighbors: number[], idx: number) => void;
    onCategoryClick: (cat: "is_raw" | "is_video" | "is_screenshot" | "is_live") => void;
    onAllPeople: () => void;
    onAllPlaces: () => void;
    onAllRecent: () => void;
}

function DiscoverySurface({
    people,
    places,
    recent,
    onSelectPerson,
    onSelectPlace,
    onSelectAsset,
    onCategoryClick,
    onAllPeople,
    onAllPlaces,
    onAllRecent,
}: DiscoveryProps) {
    return (
        <div className="kp-search-hub-discovery">
            {people.length > 0 && (
                <Section title="People" seeAll={onAllPeople}>
                    <div className="kp-foryou-carousel kp-foryou-people">
                        {people
                            .slice()
                            .sort((a, b) => b.face_count - a.face_count)
                            .slice(0, 12)
                            .map((p) => (
                                <button
                                    key={p.id}
                                    type="button"
                                    className="kp-foryou-person"
                                    onClick={() => onSelectPerson(p)}
                                >
                                    <Avatar size="lg" personId={p.id} alt={p.name ?? ""} />
                                    <span>{p.name ?? "Unnamed"}</span>
                                </button>
                            ))}
                    </div>
                </Section>
            )}

            {places.length > 0 && (
                <Section title="Places" seeAll={onAllPlaces}>
                    <div className="kp-search-hub-places">
                        {places.slice(0, 8).map((p) => (
                            <Card
                                key={p.place_id}
                                padding="none"
                                hoverable
                                onClick={() => onSelectPlace(p)}
                            >
                                {p.sample_asset_ids[0] && (
                                    <div className="kp-search-hub-place-cover">
                                        <ThumbImage
                                            assetId={p.sample_asset_ids[0]}
                                            size={384}
                                            mime="image/jpeg"
                                            alt={p.city}
                                        />
                                    </div>
                                )}
                                <div className="kp-search-hub-place-meta">
                                    <strong>{p.city}</strong>
                                    <span>{p.asset_count}</span>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            <Section title="Categories">
                <div className="kp-row" style={{ flexWrap: "wrap" }}>
                    <Chip onClick={() => onCategoryClick("is_video")}>Videos</Chip>
                    <Chip onClick={() => onCategoryClick("is_raw")}>RAW</Chip>
                    <Chip onClick={() => onCategoryClick("is_screenshot")}>
                        Screenshots
                    </Chip>
                    <Chip onClick={() => onCategoryClick("is_live")}>Live</Chip>
                </div>
            </Section>

            {recent.length > 0 && (
                <Section title="Recent imports" seeAll={onAllRecent}>
                    <div className="kp-foryou-carousel kp-foryou-thumbs">
                        {recent.slice(0, 16).map((e, idx) => (
                            <button
                                key={e.id}
                                type="button"
                                className="kp-foryou-thumb"
                                onClick={() =>
                                    onSelectAsset(
                                        e.id,
                                        recent.map((r) => r.id),
                                        idx,
                                    )
                                }
                            >
                                <ThumbImage
                                    assetId={e.id}
                                    size={256}
                                    mime={e.mime}
                                    alt=""
                                />
                            </button>
                        ))}
                    </div>
                </Section>
            )}
        </div>
    );
}

interface ResultsProps {
    query: string;
    personHits: PersonView[];
    placeHits: PlaceView[];
    tripHits: TripView[];
    albumHits: AlbumView[];
    photoHits: import("../../ipc").SearchHitView[];
    isLoading: boolean;
    onSelectPerson: (p: PersonView) => void;
    onSelectPlace: (p: PlaceView) => void;
    onSelectTrip: (t: TripView) => void;
    onSelectAlbum: (a: AlbumView) => void;
    onSelectAsset: (id: number, neighbors: number[], idx: number) => void;
}

function SearchResults({
    query,
    personHits,
    placeHits,
    tripHits,
    albumHits,
    photoHits,
    isLoading,
    onSelectPerson,
    onSelectPlace,
    onSelectTrip,
    onSelectAlbum,
    onSelectAsset,
}: ResultsProps) {
    const totalEntityHits =
        personHits.length + placeHits.length + tripHits.length + albumHits.length;

    return (
        <div className="kp-search-hub-results">
            {totalEntityHits > 0 && (
                <section className="kp-search-hub-entities">
                    <h2>Top matches</h2>
                    <div className="kp-row" style={{ flexWrap: "wrap" }}>
                        {personHits.map((p) => (
                            <EntityChip
                                key={`p-${p.id}`}
                                entity={{
                                    kind: "person",
                                    id: p.id,
                                    name: p.name,
                                }}
                                size="md"
                                onClick={() => onSelectPerson(p)}
                            />
                        ))}
                        {placeHits.map((p) => (
                            <EntityChip
                                key={`pl-${p.place_id}`}
                                entity={{
                                    kind: "place",
                                    placeId: p.place_id,
                                    name: `${p.city}, ${p.country}`,
                                }}
                                size="md"
                                onClick={() => onSelectPlace(p)}
                            />
                        ))}
                        {tripHits.map((t) => (
                            <EntityChip
                                key={`t-${t.id}`}
                                entity={{ kind: "trip", id: t.id, name: t.name }}
                                size="md"
                                onClick={() => onSelectTrip(t)}
                            />
                        ))}
                        {albumHits.map((a) => (
                            <EntityChip
                                key={`a-${a.id}`}
                                entity={{ kind: "album", id: a.id, name: a.name }}
                                size="md"
                                onClick={() => onSelectAlbum(a)}
                            />
                        ))}
                    </div>
                </section>
            )}

            <section>
                <h2>
                    Photos {query && <span>· "{query}"</span>}{" "}
                    <span className="kp-search-hub-count">
                        {photoHits.length}
                    </span>
                </h2>
                {isLoading && (
                    <p className="kp-search-hub-loading">Searching…</p>
                )}
                {!isLoading && photoHits.length === 0 && (
                    <p className="kp-search-hub-empty">
                        <ImageIcon size={20} aria-hidden /> No matching photos. Try
                        a different word, or open Filters to narrow by type or
                        camera.
                    </p>
                )}
                <div className="kp-search-hub-grid">
                    {photoHits.slice(0, 200).map((hit, idx) => (
                        <motion.button
                            key={hit.id}
                            layoutId={`asset-${hit.id}`}
                            transition={{
                                duration: 0.32,
                                ease: [0.32, 0.72, 0, 1],
                            }}
                            type="button"
                            className="kp-search-hub-cell"
                            onClick={() =>
                                onSelectAsset(
                                    hit.id,
                                    photoHits.map((h) => h.id),
                                    idx,
                                )
                            }
                        >
                            <ThumbImage
                                assetId={hit.id}
                                size={256}
                                mime={hit.mime}
                                alt=""
                            />
                        </motion.button>
                    ))}
                </div>
            </section>
        </div>
    );
}

function Section({
    title,
    children,
    seeAll,
}: {
    title: string;
    children: React.ReactNode;
    seeAll?: () => void;
}) {
    return (
        <section className="kp-foryou-section">
            <header>
                <h2>{title}</h2>
                {seeAll && (
                    <button
                        type="button"
                        className="kp-foryou-see-all"
                        onClick={seeAll}
                    >
                        See all →
                    </button>
                )}
            </header>
            {children}
        </section>
    );
}

function FilterToggleRow({
    label,
    value,
    onChange,
}: {
    label: string;
    value: boolean | null;
    onChange: (v: boolean | null) => void;
}) {
    return (
        <div className="kp-search-hub-filter-row">
            <span>{label}</span>
            <div className="kp-row">
                <Chip
                    active={value === true}
                    onClick={() => onChange(value === true ? null : true)}
                    size="sm"
                >
                    Yes
                </Chip>
                <Chip
                    active={value === false}
                    onClick={() => onChange(value === false ? null : false)}
                    size="sm"
                >
                    No
                </Chip>
            </div>
        </div>
    );
}
