import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
    Camera,
    Copy,
    FolderOpen,
    Lock,
    MapPin,
    PawPrint,
    Plane,
    Plus,
    Star,
    Users,
} from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { AlbumView } from "../../bindings/AlbumView";
import type { PersonView } from "../../bindings/PersonView";
import type { PlaceView } from "../../bindings/PlaceView";
import type { TripView } from "../../bindings/TripView";
import type { SmartAlbumView } from "../../bindings/SmartAlbumView";
import {
    Avatar,
    Button,
    Card,
    EmptyState,
    EntityChip,
    IconButton,
} from "../../components";
import ThumbImage from "../timeline/ThumbImage";
import "./albums_hub.css";

// Phase 6 AlbumsHub — replaces the flat Albums.tsx with the Apple-Photos
// "Albums" hub layout. Every kind of collection lives here:
//
//   * My Albums          — user-created, password-aware
//   * Smart albums       — rule-based, link to SmartAlbums sub-section
//   * People & Pets      — circular avatar carousel
//   * Places             — top cities, linking to PlaceDetail
//   * Trips              — auto-detected, opens as albums
//   * Media types        — Videos / Selfies / RAW / Screenshots / Live
//                          (each is just a Search scope preset)
//   * Utilities          — Duplicates, Imports
//
// Phase 6 leaves the per-kind detail screens (PersonDetail, AlbumDetail,
// SmartAlbumDetail, TripDetail) in place — Phase 9 polish will unify
// them into one CollectionDetail. The hub is the user-facing payoff,
// and the detail screens already work fine.
export default function AlbumsHub() {
    const setView = useSession((s) => s.setView);
    const pushView = useSession((s) => s.pushView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const markAlbumUnlocked = useSession((s) => s.markAlbumUnlocked);
    const queryClient = useQueryClient();

    const albums = useQuery<AlbumView[]>({
        queryKey: ["albums", hiddenUnlocked ? "withHidden" : "plain"],
        queryFn: () => api.listAlbums(hiddenUnlocked),
    });
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
    const smartAlbums = useQuery<SmartAlbumView[]>({
        queryKey: ["smart-albums"],
        queryFn: api.listSmartAlbums,
    });

    const [newName, setNewName] = useState("");
    const [newPassword, setNewPassword] = useState("");
    const [busy, setBusy] = useState(false);
    const [showNewForm, setShowNewForm] = useState(false);

    const create = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!newName.trim()) return;
        setBusy(true);
        try {
            await api.createAlbum(newName, newPassword.trim() ? newPassword : null);
            setNewName("");
            setNewPassword("");
            setShowNewForm(false);
            await queryClient.invalidateQueries({ queryKey: ["albums"] });
        } finally {
            setBusy(false);
        }
    };

    const unlockAlbum = async (album: AlbumView) => {
        const pw = window.prompt(`Password for "${album.name}"`);
        if (!pw) return;
        const ok = await api.unlockAlbum(album.id, pw);
        if (ok) {
            markAlbumUnlocked(album.id);
            await queryClient.invalidateQueries({ queryKey: ["albums"] });
        }
    };

    const userAlbums = (albums.data ?? []).filter((a) => !a.hidden);
    const hiddenAlbums = (albums.data ?? []).filter(
        (a) => a.hidden && hiddenUnlocked,
    );

    if (albums.isLoading) {
        return (
            <div className="kp-albums-hub">
                <header className="kp-albums-hub-header">
                    <h1>Albums</h1>
                </header>
                <div className="kp-library-loading">Loading…</div>
            </div>
        );
    }

    return (
        <div className="kp-albums-hub">
            <header className="kp-albums-hub-header">
                <h1>Albums</h1>
                <Button
                    variant="primary"
                    leadingIcon={<Plus size={14} />}
                    onClick={() => setShowNewForm((v) => !v)}
                >
                    New album
                </Button>
            </header>

            {showNewForm && (
                <form className="kp-albums-hub-create" onSubmit={create}>
                    <input
                        placeholder="Album name"
                        value={newName}
                        onChange={(e) => setNewName(e.target.value)}
                        disabled={busy}
                    />
                    <input
                        type="password"
                        placeholder="password (optional, hides the album)"
                        value={newPassword}
                        onChange={(e) => setNewPassword(e.target.value)}
                        disabled={busy}
                    />
                    <Button
                        type="submit"
                        variant="primary"
                        loading={busy}
                        disabled={!newName.trim()}
                    >
                        Create
                    </Button>
                    <Button
                        type="button"
                        variant="ghost"
                        onClick={() => setShowNewForm(false)}
                    >
                        Cancel
                    </Button>
                </form>
            )}

            {userAlbums.length === 0 && hiddenAlbums.length === 0 ? (
                <Card padding="none">
                    <EmptyState
                        icon={<FolderOpen size={36} />}
                        title="No albums yet"
                        hint="Create your first album above, or browse smart albums and trips below."
                    />
                </Card>
            ) : (
                <Section title="My albums" count={userAlbums.length}>
                    <div className="kp-albums-hub-grid">
                        {userAlbums.map((a) => (
                            <AlbumCard
                                key={a.id}
                                album={a}
                                onOpen={() =>
                                    pushView({
                                        kind: "album",
                                        id: a.id,
                                        name: a.name,
                                        source: "user",
                                    })
                                }
                                onUnlock={() => unlockAlbum(a)}
                            />
                        ))}
                    </div>
                </Section>
            )}

            {hiddenAlbums.length > 0 && (
                <Section
                    title="Hidden"
                    count={hiddenAlbums.length}
                    icon={<Lock size={16} />}
                >
                    <div className="kp-albums-hub-grid">
                        {hiddenAlbums.map((a) => (
                            <AlbumCard
                                key={a.id}
                                album={a}
                                onOpen={() =>
                                    pushView({
                                        kind: "album",
                                        id: a.id,
                                        name: a.name,
                                        source: "user",
                                    })
                                }
                                onUnlock={() => unlockAlbum(a)}
                            />
                        ))}
                    </div>
                </Section>
            )}

            {(smartAlbums.data ?? []).length > 0 && (
                <Section
                    title="Smart albums"
                    count={smartAlbums.data?.length ?? 0}
                    icon={<Star size={16} />}
                    seeAll={() => setView({ kind: "smart_albums" })}
                >
                    <div className="kp-albums-hub-grid">
                        {(smartAlbums.data ?? []).slice(0, 8).map((sa) => (
                            <Card
                                key={sa.id}
                                padding="md"
                                hoverable
                                onClick={() =>
                                    pushView({
                                        kind: "smart_album",
                                        id: sa.id,
                                        name: sa.name,
                                    })
                                }
                            >
                                <div className="kp-albums-hub-card-content">
                                    <Star
                                        size={20}
                                        style={{ color: "var(--color-accent-500)" }}
                                    />
                                    <strong>{sa.name}</strong>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            {(people.data ?? []).length > 0 && (
                <Section
                    title="People"
                    count={people.data?.length ?? 0}
                    icon={<Users size={16} />}
                    seeAll={() => setView({ kind: "people" })}
                >
                    <div className="kp-albums-hub-people">
                        {(people.data ?? [])
                            .slice()
                            .sort((a, b) => b.face_count - a.face_count)
                            .slice(0, 12)
                            .map((p) => (
                                <button
                                    key={p.id}
                                    type="button"
                                    className="kp-albums-hub-person"
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

            {(places.data ?? []).length > 0 && (
                <Section
                    title="Places"
                    count={places.data?.length ?? 0}
                    icon={<MapPin size={16} />}
                    seeAll={() => setView({ kind: "places" })}
                >
                    <div className="kp-albums-hub-places">
                        {(places.data ?? []).slice(0, 8).map((p) => (
                            <Card
                                key={p.place_id}
                                padding="none"
                                hoverable
                                onClick={() =>
                                    pushView({
                                        kind: "place",
                                        placeId: p.place_id,
                                        name: `${p.city}, ${p.country}`,
                                    })
                                }
                            >
                                {p.sample_asset_ids[0] && (
                                    <div className="kp-albums-hub-place-cover">
                                        <ThumbImage
                                            assetId={p.sample_asset_ids[0]}
                                            size={384}
                                            mime="image/jpeg"
                                            alt={p.city}
                                        />
                                    </div>
                                )}
                                <div className="kp-albums-hub-place-meta">
                                    <strong>{p.city}</strong>
                                    <span>{p.asset_count.toLocaleString()}</span>
                                </div>
                            </Card>
                        ))}
                    </div>
                </Section>
            )}

            {(trips.data ?? []).length > 0 && (
                <Section
                    title="Trips"
                    count={trips.data?.length ?? 0}
                    icon={<Plane size={16} />}
                    seeAll={() => setView({ kind: "trips" })}
                    actions={
                        <Button
                            variant="ghost"
                            size="sm"
                            onClick={async () => {
                                await api.detectTripsRun();
                                await queryClient.invalidateQueries({
                                    queryKey: ["trips"],
                                });
                            }}
                        >
                            Re-detect
                        </Button>
                    }
                >
                    <div className="kp-albums-hub-pills">
                        {(trips.data ?? []).slice(0, 12).map((t) => (
                            <EntityChip
                                key={t.id}
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

            <Section title="Media types" icon={<Camera size={16} />}>
                <div className="kp-albums-hub-pills">
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "search" })}
                    >
                        Videos
                    </Button>
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "search" })}
                    >
                        RAW
                    </Button>
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "search" })}
                    >
                        Screenshots
                    </Button>
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "search" })}
                    >
                        Live
                    </Button>
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "pets" })}
                        leadingIcon={<PawPrint size={14} />}
                    >
                        Pets
                    </Button>
                </div>
            </Section>

            <Section title="Utilities">
                <div className="kp-albums-hub-pills">
                    <Button
                        variant="secondary"
                        size="sm"
                        leadingIcon={<Copy size={14} />}
                        onClick={() => setView({ kind: "duplicates" })}
                    >
                        Duplicates
                    </Button>
                    <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => setView({ kind: "sources" })}
                    >
                        Sources / imports
                    </Button>
                </div>
            </Section>
        </div>
    );
}

interface SectionProps {
    title: string;
    count?: number;
    icon?: React.ReactNode;
    seeAll?: () => void;
    actions?: React.ReactNode;
    children: React.ReactNode;
}

function Section({
    title,
    count,
    icon,
    seeAll,
    actions,
    children,
}: SectionProps) {
    return (
        <section className="kp-albums-hub-section">
            <header>
                <div className="kp-albums-hub-section-title">
                    {icon}
                    <h2>{title}</h2>
                    {count != null && (
                        <span className="kp-albums-hub-section-count">{count}</span>
                    )}
                </div>
                <div className="kp-albums-hub-section-actions">
                    {actions}
                    {seeAll && (
                        <button
                            type="button"
                            className="kp-foryou-see-all"
                            onClick={seeAll}
                        >
                            See all →
                        </button>
                    )}
                </div>
            </header>
            {children}
        </section>
    );
}

interface AlbumCardProps {
    album: AlbumView;
    onOpen: () => void;
    onUnlock: () => void;
}

function AlbumCard({ album, onOpen, onUnlock }: AlbumCardProps) {
    const locked = album.has_password && !album.unlocked;
    return (
        <Card
            padding="md"
            hoverable={!locked}
            onClick={locked ? undefined : onOpen}
            className="kp-albums-hub-album"
        >
            <div className="kp-albums-hub-album-content">
                <div className="kp-albums-hub-album-icon">
                    {locked ? <Lock size={20} /> : <FolderOpen size={20} />}
                </div>
                <div className="kp-albums-hub-album-meta">
                    <strong>{album.name}</strong>
                    <span>{album.member_count} items</span>
                </div>
            </div>
            {locked && (
                <IconButton
                    icon={<Lock size={14} />}
                    label="Unlock album"
                    size="sm"
                    onClick={(e) => {
                        e.stopPropagation();
                        onUnlock();
                    }}
                />
            )}
        </Card>
    );
}
