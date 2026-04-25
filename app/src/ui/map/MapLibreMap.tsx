import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { MapPin } from "lucide-react";
import {
    Map as MapGL,
    NavigationControl,
    Source,
    Layer,
    type MapRef,
    type MapLayerMouseEvent,
} from "react-map-gl/maplibre";
import type { LngLatBoundsLike, GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import type { MapPoint } from "../../bindings/MapPoint";
import ThumbImage from "../timeline/ThumbImage";
import { Button, EmptyState } from "../../components";
import "./map.css";

// MapLibre-backed Map view — replaces the legacy SVG equirectangular
// projection with a real OSM-quality basemap. Clustering is delegated
// to MapLibre's built-in Supercluster (in a Worker), which handles up
// to ~100K markers and exposes `getClusterExpansionZoom` so a click
// either zooms in or opens the cluster sheet at max zoom.
//
// Mounted from `Shell.tsx` only when `?newmap=1`. Step 4 of the
// migration cuts over the default and removes the legacy implementation.

// Style URLs are served by the Tauri custom protocol in
// `commands/map_tiles.rs` — the webview never opens a direct HTTPS
// connection to the upstream tile CDN. Outside Tauri (e.g. browser
// dev mode) we fall back to direct fetches; the protocol handler
// only exists in the desktop runtime.
const IS_TAURI = typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in window &&
    !(window as { __MV_MOCK_IPC__?: unknown }).__MV_MOCK_IPC__;

const STYLE_LIGHT = IS_TAURI
    ? "mvtile://openfreemap/styles/liberty"
    : "https://tiles.openfreemap.org/styles/liberty";
const STYLE_DARK = IS_TAURI
    ? "mvtile://openfreemap/styles/dark"
    : "https://tiles.openfreemap.org/styles/dark";

// MapLibre fetches every resource (styles, vector tiles, glyphs,
// sprites, satellite raster) — `transformRequest` rewrites each
// upstream URL to its `mvtile://` equivalent so the Tauri proxy
// handles it. Browser dev mode (no Tauri) skips the rewrite and lets
// the webview hit upstream directly, since the custom protocol only
// exists in the desktop runtime.
type TransformResult = { url: string } | undefined;
const transformRequest = (url: string): TransformResult => {
    if (!IS_TAURI) return { url };
    if (url.startsWith("mvtile://")) return { url };
    if (url.startsWith("https://tiles.openfreemap.org/styles/")) {
        const name = url.slice("https://tiles.openfreemap.org/styles/".length);
        return { url: `mvtile://openfreemap/styles/${name}` };
    }
    if (url.startsWith("https://tiles.openfreemap.org/planet/")) {
        const suffix = url.slice("https://tiles.openfreemap.org/planet/".length);
        return { url: `mvtile://openfreemap/tiles/${suffix}` };
    }
    if (url.startsWith("https://tiles.openfreemap.org/fonts/")) {
        const suffix = url.slice("https://tiles.openfreemap.org/fonts/".length);
        return { url: `mvtile://openfreemap-fonts/${suffix}` };
    }
    if (url.startsWith("https://tiles.openfreemap.org/sprites/ofm_f384/")) {
        const suffix = url.slice(
            "https://tiles.openfreemap.org/sprites/ofm_f384/".length,
        );
        return { url: `mvtile://openfreemap-sprite/${suffix}` };
    }
    // Esri satellite — not enabled by default yet, but the rewrite
    // lands so a settings toggle in a follow-up commit can switch
    // styles without round-tripping through this file.
    const ESRI_PREFIX =
        "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/";
    if (url.startsWith(ESRI_PREFIX)) {
        const suffix = url.slice(ESRI_PREFIX.length);
        return { url: `mvtile://esri/${suffix}` };
    }
    return { url };
};

// Cluster aggregation tuning. clusterRadius is in screen pixels —
// 50 px matches the visual size of our marker so clusters of clusters
// don't visibly overlap. clusterMaxZoom: at zoom > 14 we render
// unclustered points (individual photo dots).
const CLUSTER_RADIUS = 50;
const CLUSTER_MAX_ZOOM = 14;

const SOURCE_ID = "photos";
const CLUSTERS_LAYER = "clusters";
const CLUSTER_COUNT_LAYER = "cluster-count";
const POINTS_LAYER = "unclustered-point";

function useResolvedTheme(): "light" | "dark" {
    const compute = (): "light" | "dark" => {
        if (typeof document === "undefined") return "light";
        const explicit = document.documentElement.getAttribute("data-theme");
        if (explicit === "light" || explicit === "dark") return explicit;
        if (typeof window !== "undefined" && window.matchMedia) {
            return window.matchMedia("(prefers-color-scheme: dark)").matches
                ? "dark"
                : "light";
        }
        return "light";
    };
    const [theme, setTheme] = useState<"light" | "dark">(compute);
    useEffect(() => {
        const onChange = () => setTheme(compute());
        const obs = new MutationObserver(onChange);
        obs.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-theme"],
        });
        const mq = window.matchMedia("(prefers-color-scheme: dark)");
        mq.addEventListener("change", onChange);
        return () => {
            obs.disconnect();
            mq.removeEventListener("change", onChange);
        };
    }, []);
    return theme;
}

// Compute lat/lon bounds for the dense 90% of points (5th–95th
// percentile of each axis). Outliers — a single Tokyo photo amid 600
// in Bangalore — don't pull the initial frame to a global view, but
// they're still pannable to.
function computeBounds(points: MapPoint[]): LngLatBoundsLike | null {
    if (points.length === 0) return null;
    const lats = points.map((p) => p.lat).sort((a, b) => a - b);
    const lons = points.map((p) => p.lon).sort((a, b) => a - b);
    const lo = (xs: number[]) => xs[Math.floor(xs.length * 0.05)] ?? xs[0];
    const hi = (xs: number[]) => xs[Math.floor(xs.length * 0.95)] ?? xs[xs.length - 1];
    return [
        [lo(lons), lo(lats)],
        [hi(lons), hi(lats)],
    ];
}

export default function MapLibreMap() {
    const theme = useResolvedTheme();
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const mapRef = useRef<MapRef | null>(null);

    const query = useQuery({
        queryKey: ["map-points"],
        queryFn: () => api.mapPoints(null, null, null),
    });
    const points = useMemo(() => query.data ?? [], [query.data]);

    // GeoJSON FeatureCollection for the photos source. Supercluster
    // aggregates these into clusters by zoom level inside MapLibre.
    const geojson = useMemo(() => {
        return {
            type: "FeatureCollection" as const,
            features: points.map((p) => ({
                type: "Feature" as const,
                geometry: {
                    type: "Point" as const,
                    coordinates: [p.lon, p.lat],
                },
                properties: {
                    asset_id: p.asset_id,
                    taken_at_utc_day: p.taken_at_utc_day ?? null,
                },
            })),
        };
    }, [points]);

    // The set of photos in the currently-selected cluster (rendered as
    // a bottom sheet for browsing without leaving the map).
    const [selected, setSelected] = useState<MapPoint[] | null>(null);

    // Auto-fit on first data load *after* the map has finished
    // loading. Doing it earlier (on points effect) loses the race
    // against MapLibre's initialViewState — fitBounds gets overridden
    // by the initial view animation. Tracking both `mapLoaded` and
    // `points` and only firing once keeps the user's later pan/zoom
    // sticky.
    const [mapLoaded, setMapLoaded] = useState(false);
    const fittedRef = useRef(false);
    useEffect(() => {
        if (fittedRef.current || !mapLoaded || points.length === 0) return;
        const map = mapRef.current?.getMap();
        if (!map) return;
        const bounds = computeBounds(points);
        if (!bounds) return;
        fittedRef.current = true;
        map.fitBounds(bounds, { padding: 80, duration: 0, maxZoom: 12 });
    }, [points, mapLoaded]);

    // Click handler — runs on every map click. We use the layer event
    // pattern instead of `interactiveLayerIds` props because we need
    // distinct behaviour for cluster vs single point.
    const onClick = (e: MapLayerMouseEvent) => {
        const feature = e.features?.[0];
        if (!feature) return;
        const map = mapRef.current?.getMap();
        if (!map) return;

        if (feature.layer.id === CLUSTERS_LAYER) {
            const clusterId = feature.properties?.cluster_id as number;
            const src = map.getSource(SOURCE_ID) as GeoJSONSource | undefined;
            if (!src || clusterId == null) return;
            const currentZoom = map.getZoom();
            // Below clusterMaxZoom: zoom into the cluster so it splits.
            // At/above: open the sheet listing every photo in it.
            if (currentZoom < CLUSTER_MAX_ZOOM) {
                src.getClusterExpansionZoom(clusterId).then((nextZoom) => {
                    const geom = feature.geometry as GeoJSON.Point;
                    map.easeTo({
                        center: geom.coordinates as [number, number],
                        zoom: nextZoom,
                        duration: 400,
                    });
                });
                return;
            }
            // Pull the cluster's leaves and open the sheet.
            src.getClusterLeaves(clusterId, 1000, 0).then((leaves) => {
                setSelected(
                    leaves.map((leaf) => {
                        const props = (leaf.properties ?? {}) as {
                            asset_id: number;
                            taken_at_utc_day: number | null;
                        };
                        const coords = (leaf.geometry as GeoJSON.Point)
                            .coordinates as [number, number];
                        return {
                            asset_id: props.asset_id,
                            lon: coords[0],
                            lat: coords[1],
                            taken_at_utc_day: props.taken_at_utc_day,
                        };
                    }),
                );
            });
            return;
        }
        if (feature.layer.id === POINTS_LAYER) {
            const props = (feature.properties ?? {}) as { asset_id: number };
            setView({
                kind: "asset",
                id: props.asset_id,
                back: currentView,
                neighbors: [props.asset_id],
                index: 0,
            });
        }
    };

    if (query.isLoading) {
        return (
            <div className="kp-map">
                <div className="kp-library-loading">Loading map…</div>
            </div>
        );
    }
    if (query.isError) {
        return (
            <div className="kp-map">
                <EmptyState
                    icon={<MapPin size={36} />}
                    title="Map failed to load"
                    hint="Check that the database is reachable and try again."
                />
            </div>
        );
    }
    if (points.length === 0) {
        return (
            <div className="kp-map">
                <EmptyState
                    icon={<MapPin size={36} />}
                    title="No geo-tagged photos yet"
                    hint="Photos with EXIF GPS metadata will surface here. Most modern phones write GPS automatically when location services are on."
                />
            </div>
        );
    }

    // Cluster colours — light + dark theme parity. The accent ramp
    // matches `--color-accent-{300,500,700}` from tokens.css. We pass
    // hardcoded hex here because MapLibre's expression engine doesn't
    // resolve CSS custom properties.
    const accent = theme === "dark"
        ? { lo: "#7aa2f7", mid: "#6889d8", hi: "#5169b3" }
        : { lo: "#3a72e5", mid: "#2855c4", hi: "#1c3e9a" };
    const ring = theme === "dark" ? "#0b0b0b" : "#ffffff";

    return (
        <div className="kp-map">
            <header className="kp-map-toolbar">
                <div>
                    <h1>Map</h1>
                    <p>
                        {points.length.toLocaleString()} geo-tagged
                    </p>
                </div>
            </header>
            <div className="kp-map-stage">
                <MapGL
                    ref={mapRef}
                    initialViewState={{ longitude: 0, latitude: 20, zoom: 1.5 }}
                    mapStyle={theme === "dark" ? STYLE_DARK : STYLE_LIGHT}
                    style={{ width: "100%", height: "100%" }}
                    attributionControl={{ compact: true }}
                    interactiveLayerIds={[CLUSTERS_LAYER, POINTS_LAYER]}
                    onClick={onClick}
                    onLoad={() => setMapLoaded(true)}
                    transformRequest={transformRequest}
                    cursor="grab"
                >
                    <NavigationControl position="top-right" showCompass={false} />
                    <Source
                        id={SOURCE_ID}
                        type="geojson"
                        data={geojson}
                        cluster
                        clusterRadius={CLUSTER_RADIUS}
                        clusterMaxZoom={CLUSTER_MAX_ZOOM}
                    >
                        {/* Cluster bubble — radius scales with point_count. */}
                        <Layer
                            id={CLUSTERS_LAYER}
                            type="circle"
                            filter={["has", "point_count"]}
                            paint={{
                                "circle-color": [
                                    "step",
                                    ["get", "point_count"],
                                    accent.lo,
                                    25,
                                    accent.mid,
                                    100,
                                    accent.hi,
                                ],
                                "circle-radius": [
                                    "step",
                                    ["get", "point_count"],
                                    18,
                                    25,
                                    24,
                                    100,
                                    32,
                                ],
                                "circle-stroke-width": 3,
                                "circle-stroke-color": ring,
                            }}
                        />
                        <Layer
                            id={CLUSTER_COUNT_LAYER}
                            type="symbol"
                            filter={["has", "point_count"]}
                            layout={{
                                "text-field": "{point_count_abbreviated}",
                                "text-font": ["Noto Sans Regular"],
                                "text-size": 13,
                                "text-allow-overlap": true,
                            }}
                            paint={{
                                "text-color": "#ffffff",
                            }}
                        />
                        {/* Single-photo marker — small disc at the
                          * exact GPS coordinate. No averaging means
                          * the dot lands on the actual photo's
                          * location, not a centroid drift. */}
                        <Layer
                            id={POINTS_LAYER}
                            type="circle"
                            filter={["!", ["has", "point_count"]]}
                            paint={{
                                "circle-color": accent.mid,
                                "circle-radius": 7,
                                "circle-stroke-width": 2,
                                "circle-stroke-color": ring,
                            }}
                        />
                    </Source>
                </MapGL>
            </div>
            {selected && (
                <div className="kp-map-sheet">
                    <div className="kp-map-sheet-header">
                        <strong>
                            {selected.length} {selected.length === 1 ? "photo" : "photos"}
                        </strong>
                        <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setSelected(null)}
                        >
                            Close
                        </Button>
                    </div>
                    <div className="kp-map-sheet-grid">
                        {selected.slice(0, 60).map((p, idx) => (
                            <button
                                key={p.asset_id}
                                type="button"
                                className="kp-map-sheet-cell"
                                onClick={() =>
                                    setView({
                                        kind: "asset",
                                        id: p.asset_id,
                                        back: currentView,
                                        neighbors: selected
                                            .slice(0, 60)
                                            .map((x) => x.asset_id),
                                        index: idx,
                                    })
                                }
                            >
                                <ThumbImage
                                    assetId={p.asset_id}
                                    size={256}
                                    mime="image/jpeg"
                                    alt=""
                                />
                            </button>
                        ))}
                        {selected.length > 60 && (
                            <p className="kp-map-sheet-overflow">
                                +{selected.length - 60} more
                            </p>
                        )}
                    </div>
                </div>
            )}
        </div>
    );
}
