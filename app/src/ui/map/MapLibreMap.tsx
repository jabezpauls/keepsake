import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { MapPin } from "lucide-react";
import { Map as MapGL, NavigationControl } from "react-map-gl/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import { api } from "../../ipc";
import { EmptyState } from "../../components";
import "./map.css";

// New Map view backed by MapLibre GL JS — replaces the legacy SVG
// equirectangular projection that had no labels, no roads, and forced
// users to interpret photo positions against a featureless white blob.
//
// Step 1 of the migration ships only the basemap. Step 2 will add
// clusters + markers; step 3 adds the Tauri tile proxy + offline
// PMTiles fallback; step 4 cuts over the default and removes the
// legacy SVG implementation.
//
// Mounted from `Shell.tsx` only when the URL has `?newmap=1`. Old
// `MapView.tsx` stays the default until step 4.

// OpenFreeMap is OSM-derived, free, key-less, and ships in a couple of
// curated styles. Liberty is colourful + dense (Apple Maps-ish);
// Positron is the calm light style; the dark variant gives us automatic
// theme parity with the rest of the app.
const STYLE_LIGHT = "https://tiles.openfreemap.org/styles/liberty";
const STYLE_DARK = "https://tiles.openfreemap.org/styles/dark";

// Reads the current `data-theme` attribute on <html> (set by
// `appearance.ts`). When `auto`, falls back to OS preference. Re-runs
// when the attribute mutates so the map style swaps live.
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

export default function MapLibreMap() {
    const theme = useResolvedTheme();
    const query = useQuery({
        queryKey: ["map-points"],
        queryFn: () => api.mapPoints(null, null, null),
    });
    const points = query.data ?? [];

    // Pick a sensible initial frame: the centroid of the points, or the
    // world if none. Step 2 will replace this with a proper bounds-fit
    // once the cluster layer is wired.
    const initialView = (() => {
        if (points.length === 0) return { longitude: 0, latitude: 20, zoom: 1.5 };
        const cLon = points.reduce((s, p) => s + p.lon, 0) / points.length;
        const cLat = points.reduce((s, p) => s + p.lat, 0) / points.length;
        return { longitude: cLon, latitude: cLat, zoom: 4 };
    })();

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

    return (
        <div className="kp-map">
            <header className="kp-map-toolbar">
                <div>
                    <h1>Map</h1>
                    <p>{points.length.toLocaleString()} geo-tagged</p>
                </div>
            </header>
            <div className="kp-map-stage">
                <MapGL
                    initialViewState={initialView}
                    mapStyle={theme === "dark" ? STYLE_DARK : STYLE_LIGHT}
                    style={{ width: "100%", height: "100%" }}
                    attributionControl={{ compact: true }}
                >
                    <NavigationControl position="top-right" showCompass={false} />
                </MapGL>
            </div>
        </div>
    );
}
