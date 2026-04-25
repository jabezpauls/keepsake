import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { motion } from "framer-motion";
import { MapPin } from "lucide-react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { PlaceView } from "../../bindings/PlaceView";
import { Button, EmptyState, EntityChip } from "../../components";
import ThumbImage from "../timeline/ThumbImage";
import "./places.css";

interface Props {
    placeId: string;
    name: string;
}

// Phase 4 PlaceDetail — drilldown for a single city. Reuses the same
// list_places call so we get the centroid + sample_asset_ids without
// adding another backend command. Phase 6 will probably move this to
// CollectionDetail with `source: "place"` once that abstraction lands.
export default function PlaceDetail({ placeId, name }: Props) {
    const popView = useSession((s) => s.popView);
    const pushView = useSession((s) => s.pushView);
    const currentView = useSession((s) => s.view);

    const { data: places = [], isLoading } = useQuery<PlaceView[]>({
        queryKey: ["places"],
        queryFn: () => api.listPlaces(),
    });

    const place = useMemo(
        () => places.find((p) => p.place_id === placeId) ?? null,
        [places, placeId],
    );

    if (isLoading) {
        return (
            <div className="kp-place-detail">
                <div className="kp-library-loading">Loading…</div>
            </div>
        );
    }

    if (!place) {
        return (
            <div className="kp-place-detail">
                <EmptyState
                    icon={<MapPin size={36} />}
                    title={name}
                    hint="No photos here yet, or the place was removed since you last saw it."
                    actions={
                        <Button variant="ghost" onClick={() => popView()}>
                            ← Back
                        </Button>
                    }
                />
            </div>
        );
    }

    return (
        <div className="kp-place-detail">
            <header className="kp-place-detail-hero">
                <div className="kp-place-detail-cover">
                    {place.sample_asset_ids.slice(0, 3).map((aid) => (
                        <div key={aid} className="kp-place-detail-cover-cell">
                            <ThumbImage
                                assetId={aid}
                                size={512}
                                mime="image/jpeg"
                                alt={place.city}
                            />
                        </div>
                    ))}
                </div>
                <div className="kp-place-detail-meta">
                    <h1 className="kp-place-detail-title">{place.city}</h1>
                    <p className="kp-place-detail-subtitle">
                        {place.region && place.region !== place.city
                            ? `${place.region} · ${place.country}`
                            : place.country}
                    </p>
                    <p className="kp-place-detail-count">
                        {place.asset_count.toLocaleString()} photos · centred at{" "}
                        {place.centroid_lat.toFixed(2)},{" "}
                        {place.centroid_lon.toFixed(2)}
                    </p>
                    <div className="kp-place-detail-chips">
                        <EntityChip
                            entity={{
                                kind: "category",
                                key: "raw",
                                label: `${place.country}`,
                            }}
                            onClick={() => pushView({ kind: "search" })}
                        />
                    </div>
                </div>
            </header>

            <div className="kp-place-detail-grid">
                {place.sample_asset_ids.map((aid, idx) => (
                    <motion.button
                        key={aid}
                        layoutId={`asset-${aid}`}
                        type="button"
                        className="kp-place-detail-cell"
                        transition={{
                            duration: 0.32,
                            ease: [0.32, 0.72, 0, 1],
                        }}
                        onClick={() =>
                            pushView({
                                kind: "asset",
                                id: aid,
                                back: currentView,
                                neighbors: place.sample_asset_ids,
                                index: idx,
                            })
                        }
                    >
                        <ThumbImage
                            assetId={aid}
                            size={512}
                            mime="image/jpeg"
                            alt=""
                        />
                    </motion.button>
                ))}
            </div>

            <p
                style={{
                    color: "var(--color-text-tertiary)",
                    font: "var(--font-caption)",
                    marginTop: "var(--space-5)",
                }}
            >
                Showing {place.sample_asset_ids.length} of{" "}
                {place.asset_count.toLocaleString()} photos. A full grid lands
                in Phase 6 when CollectionDetail unifies place / album /
                person / smart-album views.
            </p>
        </div>
    );
}
