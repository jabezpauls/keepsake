import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Plus, Sparkles } from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { MemoryGroupView } from "../../bindings/MemoryGroupView";
import type { YearInPhotosView } from "../../bindings/YearInPhotosView";
import type { PersonYearMemoryView } from "../../bindings/PersonYearMemoryView";
import type { PersonView } from "../../bindings/PersonView";
import type { TripView } from "../../bindings/TripView";
import type { PlaceView } from "../../bindings/PlaceView";
import type { TimelineEntryView } from "../../bindings/TimelineEntryView";
import type { IncomingShareView } from "../../bindings/IncomingShareView";
import {
    Avatar,
    Button,
    Card,
    EmptyState,
    EntityChip,
    IconButton,
} from "../../components";
import ThumbImage from "../timeline/ThumbImage";
import "./for_you.css";

// Phase 5 For-You — the new default landing surface. Vertical scroll of
// horizontal carousels, each backed by an existing analytics command:
//
//   * On this day                  → memoriesOnThisDay
//   * Year in photos               → memoriesYearInPhotos
//   * People across the years      → memoriesPersonYear(min=3)
//   * Recent trips                 → listTrips (sorted by created_at desc)
//   * Featured places              → listPlaces (top 8 by asset_count)
//   * Featured people              → listPeople (top 8 by face_count)
//   * Sharing activity             → listIncomingShares
//
// Tapping a Memory hero opens MemorySlideshow — full-bleed Radix Dialog
// playing a Ken-Burns crossfade through the asset list. No video
// synthesis; reduced-motion turns the slideshow into a paged grid.
export default function ForYou() {
    const setView = useSession((s) => s.setView);
    const pushView = useSession((s) => s.pushView);

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
    const trips = useQuery<TripView[]>({
        queryKey: ["trips"],
        queryFn: api.listTrips,
    });
    const places = useQuery<PlaceView[]>({
        queryKey: ["places"],
        queryFn: () => api.listPlaces(),
    });
    const recentImports = useQuery<TimelineEntryView[]>({
        queryKey: ["timeline", "recent-imports"],
        queryFn: async () => {
            const page = await api.timelinePage(null, 30);
            return page.entries;
        },
    });
    const incomingShares = useQuery<IncomingShareView[]>({
        queryKey: ["incoming-shares"],
        queryFn: api.listIncomingShares,
    });

    const personNameById = new Map<number, string>();
    for (const p of people.data ?? []) {
        if (p.name) personNameById.set(p.id, p.name);
    }

    const today = new Date();
    const todayLabel = today.toLocaleDateString(undefined, {
        weekday: "long",
        month: "long",
        day: "numeric",
    });

    const [slideshow, setSlideshow] = useState<MemoryGroupView | null>(null);

    const isAllEmpty =
        !groups.data?.length &&
        !years.data?.length &&
        !personYears.data?.length &&
        !trips.data?.length &&
        !places.data?.length &&
        !recentImports.data?.length;

    if (
        groups.isLoading ||
        years.isLoading ||
        personYears.isLoading ||
        trips.isLoading
    ) {
        return (
            <div className="kp-foryou">
                <header className="kp-foryou-header">
                    <h1>For You</h1>
                    <p>Loading your library…</p>
                </header>
            </div>
        );
    }

    if (isAllEmpty) {
        return (
            <div className="kp-foryou">
                <EmptyState
                    icon={<Sparkles size={36} />}
                    title="Add a source to start your library"
                    hint="Once photos are imported, this page surfaces memories from past years, recent trips, and the people you spend time with."
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
        <div className="kp-foryou">
            <header className="kp-foryou-header">
                <p className="kp-foryou-eyebrow">{todayLabel}</p>
                <h1>For You</h1>
            </header>

            {(groups.data ?? []).length > 0 && (
                <Section title="On this day">
                    <div className="kp-foryou-carousel">
                        {(groups.data ?? []).map((g) => (
                            <Card
                                key={`onday-${g.year}`}
                                padding="none"
                                hoverable
                                onClick={() => setSlideshow(g)}
                                className="kp-foryou-hero"
                            >
                                <div className="kp-foryou-hero-cover">
                                    <ThumbImage
                                        assetId={g.asset_ids[0] ?? 0}
                                        size={1024}
                                        mime="image/jpeg"
                                        alt=""
                                    />
                                </div>
                                <div className="kp-foryou-hero-meta">
                                    <strong>
                                        {g.years_ago === 1
                                            ? "1 year ago"
                                            : `${g.years_ago} years ago`}
                                    </strong>
                                    <span>
                                        {g.year} · {g.asset_ids.length}{" "}
                                        {g.asset_ids.length === 1 ? "photo" : "photos"}
                                    </span>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            {(years.data ?? []).length > 0 && (
                <Section title="Year in photos">
                    <div className="kp-foryou-carousel">
                        {(years.data ?? []).map((y) => (
                            <Card
                                key={`year-${y.year}`}
                                padding="none"
                                hoverable
                                onClick={() =>
                                    setSlideshow({
                                        year: y.year,
                                        years_ago: today.getFullYear() - y.year,
                                        asset_ids: y.highlights,
                                    })
                                }
                                className="kp-foryou-hero kp-foryou-hero-tall"
                            >
                                <div className="kp-foryou-hero-cover">
                                    <ThumbImage
                                        assetId={y.highlights[0] ?? 0}
                                        size={1024}
                                        mime="image/jpeg"
                                        alt=""
                                    />
                                </div>
                                <div className="kp-foryou-hero-meta">
                                    <strong>{y.year}</strong>
                                    <span>{y.asset_count.toLocaleString()} photos</span>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            {(personYears.data ?? []).length > 0 && (
                <Section title="People across the years">
                    <div className="kp-foryou-carousel">
                        {(personYears.data ?? []).map((pm) => {
                            const name =
                                personNameById.get(pm.person_id) ??
                                `Person #${pm.person_id}`;
                            return (
                                <Card
                                    key={`py-${pm.person_id}-${pm.year}`}
                                    padding="none"
                                    hoverable
                                    onClick={() =>
                                        setSlideshow({
                                            year: pm.year,
                                            years_ago: today.getFullYear() - pm.year,
                                            asset_ids: pm.asset_ids,
                                        })
                                    }
                                    className="kp-foryou-hero"
                                >
                                    <div className="kp-foryou-hero-cover">
                                        <ThumbImage
                                            assetId={pm.asset_ids[0] ?? 0}
                                            size={1024}
                                            mime="image/jpeg"
                                            alt=""
                                        />
                                    </div>
                                    <div className="kp-foryou-hero-meta">
                                        <strong>{name} · {pm.year}</strong>
                                        <span>{pm.asset_ids.length} photos</span>
                                    </div>
                                </Card>
                            );
                        })}
                    </div>
                </Section>
            )}

            {(trips.data ?? []).length > 0 && (
                <Section
                    title="Recent trips"
                    seeAll={() => setView({ kind: "trips" })}
                >
                    <div className="kp-foryou-carousel">
                        {(trips.data ?? []).slice(0, 8).map((t) => (
                            <EntityChip
                                key={`trip-${t.id}`}
                                entity={{ kind: "trip", id: t.id, name: t.name }}
                                size="md"
                                onClick={() =>
                                    pushView({
                                        kind: "album",
                                        id: t.id,
                                        name: t.name,
                                        source: "trip",
                                    })
                                }
                            />
                        ))}
                    </div>
                </Section>
            )}

            {(places.data ?? []).length > 0 && (
                <Section
                    title="Featured places"
                    seeAll={() => setView({ kind: "places" })}
                >
                    <div className="kp-foryou-carousel">
                        {(places.data ?? []).slice(0, 8).map((p) => (
                            <Card
                                key={`place-${p.place_id}`}
                                padding="none"
                                hoverable
                                onClick={() =>
                                    pushView({
                                        kind: "place",
                                        placeId: p.place_id,
                                        name: `${p.city}, ${p.country}`,
                                    })
                                }
                                className="kp-foryou-pill-card"
                            >
                                {p.sample_asset_ids[0] && (
                                    <div className="kp-foryou-pill-cover">
                                        <ThumbImage
                                            assetId={p.sample_asset_ids[0]}
                                            size={256}
                                            mime="image/jpeg"
                                            alt={p.city}
                                        />
                                    </div>
                                )}
                                <div className="kp-foryou-pill-meta">
                                    <strong>{p.city}</strong>
                                    <span>{p.asset_count.toLocaleString()}</span>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            {(people.data ?? []).length > 0 && (
                <Section
                    title="Featured people"
                    seeAll={() => setView({ kind: "people" })}
                >
                    <div className="kp-foryou-carousel kp-foryou-people">
                        {(people.data ?? [])
                            .slice()
                            .sort((a, b) => b.face_count - a.face_count)
                            .slice(0, 12)
                            .map((p) => (
                                <button
                                    key={`person-${p.id}`}
                                    type="button"
                                    className="kp-foryou-person"
                                    onClick={() =>
                                        pushView({
                                            kind: "person",
                                            id: p.id,
                                            name: p.name,
                                        })
                                    }
                                >
                                    <Avatar size="lg" personId={p.id} alt={p.name ?? ""} />
                                    <span>{p.name ?? "Unnamed"}</span>
                                </button>
                            ))}
                    </div>
                </Section>
            )}

            {(recentImports.data ?? []).length > 0 && (
                <Section
                    title="Recent imports"
                    seeAll={() => setView({ kind: "library" })}
                >
                    <div className="kp-foryou-carousel kp-foryou-thumbs">
                        {(recentImports.data ?? []).slice(0, 16).map((e, idx) => (
                            <button
                                key={e.id}
                                type="button"
                                className="kp-foryou-thumb"
                                onClick={() =>
                                    pushView({
                                        kind: "asset",
                                        id: e.id,
                                        back: { kind: "for-you" },
                                        neighbors: (recentImports.data ?? []).map(
                                            (a) => a.id,
                                        ),
                                        index: idx,
                                    })
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

            {(incomingShares.data ?? []).length > 0 && (
                <Section title="Sharing activity">
                    <div className="kp-foryou-carousel">
                        {(incomingShares.data ?? []).slice(0, 6).map((s) => (
                            <Card
                                key={`share-${s.collection_id}`}
                                padding="md"
                                hoverable
                                onClick={() => setView({ kind: "peers" })}
                            >
                                <strong>{s.album_name ?? "Shared album"}</strong>
                                <p
                                    style={{
                                        margin: "var(--space-1) 0 0 0",
                                        font: "var(--font-caption)",
                                        color: "var(--color-text-tertiary)",
                                    }}
                                >
                                    From {s.sender_identity_pub_hex.slice(0, 8)}…
                                </p>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            <MemorySlideshow
                memory={slideshow}
                onClose={() => setSlideshow(null)}
            />
        </div>
    );
}

interface SectionProps {
    title: string;
    children: React.ReactNode;
    seeAll?: () => void;
}

function Section({ title, children, seeAll }: SectionProps) {
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

interface MemorySlideshowProps {
    memory: MemoryGroupView | null;
    onClose: () => void;
}

// Memory slideshow — full-bleed modal with Ken-Burns crossfade. 4s per
// photo; spacebar pauses, arrow keys step. Reduced-motion turns it into
// a paged grid (handled by tokens.css collapsing the animation duration
// to 0).
function MemorySlideshow({ memory, onClose }: MemorySlideshowProps) {
    const [idx, setIdx] = useState(0);
    const [paused, setPaused] = useState(false);

    // Re-arm keyboard handlers and reset progress whenever the slideshow
    // re-opens with a different memory.
    useEffect(() => {
        if (!memory) return;
        setIdx(0);
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
            if (e.key === "ArrowRight")
                setIdx((i) => Math.min(memory.asset_ids.length - 1, i + 1));
            if (e.key === "ArrowLeft") setIdx((i) => Math.max(0, i - 1));
            if (e.key === " ") {
                e.preventDefault();
                setPaused((p) => !p);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [memory, onClose]);

    if (!memory) return null;

    return (
        <AnimatePresence>
            <motion.div
                className="kp-memory-slideshow"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                onClick={onClose}
            >
                <header className="kp-memory-slideshow-chrome">
                    <span>
                        {memory.year} ·{" "}
                        {memory.years_ago === 1
                            ? "1 year ago"
                            : `${memory.years_ago} years ago`}
                    </span>
                    <IconButton
                        icon={<span style={{ fontSize: 20 }}>×</span>}
                        label="Close slideshow"
                        onClick={onClose}
                    />
                </header>
                <div className="kp-memory-slideshow-stage">
                    <SlideshowImage
                        assetId={memory.asset_ids[idx] ?? 0}
                        paused={paused}
                        durationMs={4000}
                        onAdvance={() => {
                            if (paused) return;
                            setIdx((i) =>
                                i + 1 < memory.asset_ids.length ? i + 1 : 0,
                            );
                        }}
                    />
                </div>
                <div className="kp-memory-slideshow-progress">
                    {memory.asset_ids.map((_, i) => (
                        <span
                            key={i}
                            className="kp-memory-slideshow-tick"
                            data-active={i === idx ? "true" : undefined}
                        />
                    ))}
                </div>
            </motion.div>
        </AnimatePresence>
    );
}

// Single Ken-Burns slide — image scales from 1.05× → 1.00× over the
// configured duration, opacity crossfade on key change. The parent
// MemorySlideshow re-keys on each `assetId` change so framer-motion
// re-runs the animation.
function SlideshowImage({
    assetId,
    paused,
    durationMs,
    onAdvance,
}: {
    assetId: number;
    paused: boolean;
    durationMs: number;
    onAdvance: () => void;
}) {
    useEffect(() => {
        if (paused) return;
        const t = window.setTimeout(onAdvance, durationMs);
        return () => window.clearTimeout(t);
    }, [assetId, paused, durationMs, onAdvance]);

    return (
        <AnimatePresence mode="wait">
            <motion.div
                key={assetId}
                className="kp-memory-slideshow-img"
                initial={{ opacity: 0, scale: 1.05 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.6, ease: [0.32, 0.72, 0, 1] }}
            >
                <ThumbImage
                    assetId={assetId}
                    size={1024}
                    mime="image/jpeg"
                    alt=""
                />
            </motion.div>
        </AnimatePresence>
    );
}
