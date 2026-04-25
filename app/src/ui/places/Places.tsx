import { useQuery } from "@tanstack/react-query";
import { MapPin } from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { PlaceView } from "../../bindings/PlaceView";
import { Card, EmptyState } from "../../components";
import ThumbImage from "../timeline/ThumbImage";
import "./places.css";

// Phase 4 — Places list. Sidebar Pinned → Places lands here. Each
// card opens PlaceDetail (drilldown into that city's photos). The
// data comes from `list_places` which reverse-geocodes every
// GPS-tagged asset against the embedded cities500 dataset.
export default function Places() {
    const setView = useSession((s) => s.setView);
    const pushView = useSession((s) => s.pushView);

    const { data: places = [], isLoading } = useQuery<PlaceView[]>({
        queryKey: ["places"],
        queryFn: () => api.listPlaces(),
    });

    if (isLoading) {
        return (
            <div className="kp-places">
                <div className="kp-library-loading">Loading places…</div>
            </div>
        );
    }

    if (places.length === 0) {
        return (
            <div className="kp-places">
                <EmptyState
                    icon={<MapPin size={36} />}
                    title="No places yet"
                    hint="Photos with GPS metadata will be reverse-geocoded against ~80 bundled world cities. Anywhere outside that set surfaces on the Map view."
                />
            </div>
        );
    }

    return (
        <div className="kp-places">
            <header className="kp-places-header">
                <h1 className="kp-places-title">Places</h1>
                <p className="kp-places-subtitle">
                    {places.length} {places.length === 1 ? "place" : "places"} ·{" "}
                    {places.reduce((sum, p) => sum + p.asset_count, 0).toLocaleString()}{" "}
                    photos
                </p>
            </header>
            <div className="kp-places-grid">
                {places.map((p) => (
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
                        <PlaceCardCover place={p} />
                        <div className="kp-place-card-body">
                            <h3 className="kp-place-card-title">{p.city}</h3>
                            <p className="kp-place-card-subtitle">
                                {p.region && p.region !== p.city
                                    ? `${p.region} · ${p.country}`
                                    : p.country}
                            </p>
                            <p className="kp-place-card-count">
                                {p.asset_count.toLocaleString()} photos
                            </p>
                        </div>
                    </Card>
                ))}
            </div>
            <button
                type="button"
                className="kp-places-back"
                onClick={() => setView({ kind: "library" })}
            >
                ← Back to Library
            </button>
        </div>
    );
}

function PlaceCardCover({ place }: { place: PlaceView }) {
    if (place.sample_asset_ids.length === 0) {
        return <div className="kp-place-card-cover kp-place-card-cover-empty" />;
    }
    return (
        <div className="kp-place-card-cover">
            <ThumbImage
                assetId={place.sample_asset_ids[0]}
                size={512}
                mime="image/jpeg"
                alt={place.city}
            />
        </div>
    );
}
