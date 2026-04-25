import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Maximize2, MapPin, Minus, Plus } from "lucide-react";
import { feature } from "topojson-client";
import { geoEquirectangular, geoPath } from "d3-geo";
import type { Topology } from "topojson-specification";
import type { Feature, GeometryObject } from "geojson";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import ThumbImage from "../timeline/ThumbImage";
import type { MapPoint } from "../../bindings/MapPoint";
import { Button, EmptyState, IconButton } from "../../components";
// Natural Earth land-110m — landmass-only TopoJSON, ~55 KB. Bundled
// statically by Vite so the map renders fully offline (Tauri invariant).
import landTopo from "world-atlas/land-110m.json";
import "./map.css";

// Equirectangular projection: 720 px ≈ 360° lon, 360 px ≈ 180° lat.
// Keeping the viewBox dimensions constant lets us drive zoom by scaling
// it (shrink the box = zoom in) and pan by translating cx/cy.
const W = 720;
const H = 360;

// Pre-compute the land path once. d3-geo's equirectangular at
// scale = W/(2π) ≈ 114.59 gives one full 360° wrap across W pixels,
// which matches our scatter projection (lon/360 * W). The path string
// is identical regardless of zoom — vector strokes scale automatically.
const landFeature = feature(
    landTopo as unknown as Topology,
    (landTopo as unknown as Topology).objects.land,
) as unknown as Feature<GeometryObject>;
const landProjection = geoEquirectangular()
    .scale(W / (2 * Math.PI))
    .translate([W / 2, H / 2]);
const landPath = geoPath(landProjection)(landFeature) ?? "";

interface Viewport {
    /** Centre of the visible window in projection units (0,0 = world centre). */
    cx: number;
    cy: number;
    /** Multiplicative zoom — 1 fits the entire world; 16 = continent-level. */
    zoom: number;
}

const INITIAL_VIEWPORT: Viewport = { cx: 0, cy: 0, zoom: 1 };

// Phase-9 polish for the legacy Map view. Three real bugs that broke the
// experience on a vault with hundreds of geo-tagged photos:
//
//   1. World view dumped every photo into one tiny corner cluster. Fix:
//      auto-fit the viewport to the points' bounding box on first load.
//   2. No way to pan. Fix: mouse-drag pan (and `0` to reset).
//   3. Clusters overlapped at low zoom because grid bucketing alone
//      doesn't account for visual radius. Fix: post-cluster collision
//      merge — neighbours within 2× radius collapse into one bubble.
//
// Visual rebuild uses design tokens so the canvas matches the rest of
// the app instead of the hardcoded #121822 / #283044 palette.
export default function MapView() {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const query = useQuery({
        queryKey: ["map-points"],
        queryFn: () => api.mapPoints(null, null, null),
    });

    const [viewport, setViewport] = useState<Viewport>(INITIAL_VIEWPORT);
    const [selected, setSelected] = useState<MapPoint[] | null>(null);
    const svgRef = useRef<SVGSVGElement | null>(null);
    const fitOnceRef = useRef(false);

    const points = useMemo(() => query.data ?? [], [query.data]);

    // First-load auto-fit: compute the lat/lon extent of every point and
    // set the viewport so the bounding box fills 80% of the canvas. Only
    // runs once per mount — manual pan/zoom afterwards is sticky.
    useEffect(() => {
        if (fitOnceRef.current || points.length === 0) return;
        fitOnceRef.current = true;
        setViewport(fitToPoints(points));
    }, [points]);

    // Cluster: grid-bucket then collision-merge.
    const clusters = useMemo(() => {
        if (!points.length) return [];
        // Grid size in degrees scales inversely with zoom — at world view
        // we bucket 12° tiles, at zoom 16 we're down to 0.75°.
        const gridDeg = 12 / Math.max(viewport.zoom, 1);
        const buckets = new Map<string, MapPoint[]>();
        for (const p of points) {
            const gx = Math.floor(p.lon / gridDeg);
            const gy = Math.floor(p.lat / gridDeg);
            const key = `${gx}:${gy}`;
            const arr = buckets.get(key) ?? [];
            arr.push(p);
            buckets.set(key, arr);
        }
        const raw: ClusterRaw[] = Array.from(buckets.values()).map((pts) => ({
            points: pts,
            avgLat: pts.reduce((s, p) => s + p.lat, 0) / pts.length,
            avgLon: pts.reduce((s, p) => s + p.lon, 0) / pts.length,
        }));
        return mergeOverlapping(raw, viewport.zoom);
    }, [points, viewport.zoom]);

    // Wheel zoom + mouse-drag pan. The SVG only mounts after the data
    // query resolves (loading/empty states render placeholder divs), so
    // we re-attach handlers any time the rendered SVG identity changes —
    // a callback ref does this cleanly without a fragile effect dep.
    const setSvgRef = useCallback((el: SVGSVGElement | null) => {
        // Detach any previous handlers when the ref changes.
        if (svgRef.current && (svgRef.current as SVGSVGElement & {
            __mvCleanup?: () => void;
        }).__mvCleanup) {
            (svgRef.current as SVGSVGElement & { __mvCleanup?: () => void })
                .__mvCleanup!();
        }
        svgRef.current = el;
        if (!el) return;

        const onWheel = (e: WheelEvent) => {
            e.preventDefault();
            const factor = e.deltaY < 0 ? 1.2 : 1 / 1.2;
            setViewport((v) => zoomAt(v, el, e.clientX, e.clientY, factor));
        };
        let dragging = false;
        let lastX = 0;
        let lastY = 0;
        const onDown = (e: MouseEvent) => {
            if (e.button !== 0) return;
            dragging = true;
            lastX = e.clientX;
            lastY = e.clientY;
            el.style.cursor = "grabbing";
        };
        const onMove = (e: MouseEvent) => {
            if (!dragging) return;
            const rect = el.getBoundingClientRect();
            const dx = (e.clientX - lastX) / rect.width;
            const dy = (e.clientY - lastY) / rect.height;
            lastX = e.clientX;
            lastY = e.clientY;
            setViewport((v) => ({
                ...v,
                cx: v.cx - (dx * W) / v.zoom,
                cy: v.cy - (dy * H) / v.zoom,
            }));
        };
        const onUp = () => {
            if (!dragging) return;
            dragging = false;
            el.style.cursor = "grab";
        };

        el.addEventListener("wheel", onWheel, { passive: false });
        el.addEventListener("mousedown", onDown);
        window.addEventListener("mousemove", onMove);
        window.addEventListener("mouseup", onUp);

        (el as SVGSVGElement & { __mvCleanup?: () => void }).__mvCleanup = () => {
            el.removeEventListener("wheel", onWheel);
            el.removeEventListener("mousedown", onDown);
            window.removeEventListener("mousemove", onMove);
            window.removeEventListener("mouseup", onUp);
        };
    }, []);

    // Keyboard shortcuts: +/- to zoom, 0 to reset, R for reset (alias).
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (
                e.target instanceof HTMLInputElement ||
                e.target instanceof HTMLTextAreaElement
            ) {
                return;
            }
            if (e.key === "+" || e.key === "=") {
                setViewport((v) => ({ ...v, zoom: clampZoom(v.zoom * 1.4) }));
            } else if (e.key === "-" || e.key === "_") {
                setViewport((v) => ({ ...v, zoom: clampZoom(v.zoom / 1.4) }));
            } else if (e.key === "0") {
                setViewport(points.length ? fitToPoints(points) : INITIAL_VIEWPORT);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [points]);

    const project = useCallback(
        (lat: number, lon: number) => ({
            x: W / 2 + (lon / 360) * W,
            y: H / 2 - (lat / 180) * H,
        }),
        [],
    );

    const boxW = W / viewport.zoom;
    const boxH = H / viewport.zoom;
    const viewBox = `${W / 2 - boxW / 2 + viewport.cx} ${
        H / 2 - boxH / 2 + viewport.cy
    } ${boxW} ${boxH}`;

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
                    <p>
                        {points.length.toLocaleString()} geo-tagged · zoom{" "}
                        {viewport.zoom.toFixed(1)}×
                    </p>
                </div>
                <div className="kp-map-controls">
                    <IconButton
                        icon={<Plus size={14} />}
                        label="Zoom in (+)"
                        size="sm"
                        onClick={() =>
                            setViewport((v) => ({ ...v, zoom: clampZoom(v.zoom * 1.4) }))
                        }
                    />
                    <IconButton
                        icon={<Minus size={14} />}
                        label="Zoom out (−)"
                        size="sm"
                        onClick={() =>
                            setViewport((v) => ({ ...v, zoom: clampZoom(v.zoom / 1.4) }))
                        }
                    />
                    <Button
                        variant="ghost"
                        size="sm"
                        leadingIcon={<Maximize2 size={14} />}
                        onClick={() => setViewport(fitToPoints(points))}
                    >
                        Fit
                    </Button>
                </div>
            </header>
            <svg
                ref={setSvgRef}
                className="kp-map-canvas"
                viewBox={viewBox}
                preserveAspectRatio="xMidYMid meet"
                style={{ cursor: "grab" }}
            >
                <rect
                    x={-W * 4}
                    y={-H * 4}
                    width={W * 9}
                    height={H * 9}
                    fill="var(--map-bg)"
                />

                {/* Natural Earth landmass — drawn 3× across the longitudinal
                 * axis so panning past the antimeridian still shows continents
                 * instead of empty void. Vector strokes auto-scale with zoom. */}
                {[-W, 0, W].map((dx) => (
                    <path
                        key={`land-${dx}`}
                        d={landPath}
                        transform={`translate(${dx}, 0)`}
                        fill="var(--map-land)"
                        stroke="var(--map-land-stroke)"
                        strokeWidth={0.5 / viewport.zoom}
                        strokeLinejoin="round"
                    />
                ))}

                {/* Latitude / longitude graticule — every 30° in lat, 60° in lon. */}
                {[-60, -30, 0, 30, 60].map((lat) => {
                    const { y } = project(lat, 0);
                    return (
                        <line
                            key={`lat-${lat}`}
                            x1={-W * 4}
                            y1={y}
                            x2={W * 5}
                            y2={y}
                            stroke="var(--map-grid)"
                            strokeWidth={0.4 / viewport.zoom}
                        />
                    );
                })}
                {[-180, -120, -60, 0, 60, 120, 180].map((lon) => {
                    const { x } = project(0, lon);
                    return (
                        <line
                            key={`lon-${lon}`}
                            x1={x}
                            y1={-H * 4}
                            x2={x}
                            y2={H * 5}
                            stroke="var(--map-grid)"
                            strokeWidth={0.4 / viewport.zoom}
                        />
                    );
                })}

                {/* Equator highlight. */}
                <line
                    x1={-W * 4}
                    y1={H / 2}
                    x2={W * 5}
                    y2={H / 2}
                    stroke="var(--map-grid-strong)"
                    strokeWidth={0.6 / viewport.zoom}
                />

                {clusters.map((c, i) => {
                    const { x, y } = project(c.avgLat, c.avgLon);
                    const r = clusterRadius(c.points.length, viewport.zoom);
                    return (
                        <g
                            key={i}
                            onClick={(e) => {
                                e.stopPropagation();
                                setSelected(c.points);
                            }}
                            style={{ cursor: "pointer" }}
                        >
                            {/* Glow ring */}
                            <circle
                                cx={x}
                                cy={y}
                                r={r * 1.6}
                                fill="var(--color-accent-500)"
                                opacity={0.18}
                            />
                            <circle
                                cx={x}
                                cy={y}
                                r={r}
                                fill="var(--color-accent-500)"
                                opacity={0.92}
                                stroke="rgba(255,255,255,0.85)"
                                strokeWidth={r * 0.08}
                            />
                            <text
                                x={x}
                                y={y + r * 0.35}
                                textAnchor="middle"
                                fill="#fff"
                                fontSize={r * 0.95}
                                fontWeight="600"
                                style={{ pointerEvents: "none", userSelect: "none" }}
                            >
                                {c.points.length}
                            </text>
                        </g>
                    );
                })}
            </svg>
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

interface ClusterRaw {
    points: MapPoint[];
    avgLat: number;
    avgLon: number;
}

// Compute the viewport that fits every point with 10% margin. Keeps the
// canvas centred on the bounding-box centroid; zoom picks whichever axis
// is the binding constraint.
function fitToPoints(points: MapPoint[]): Viewport {
    let minLat = 90;
    let maxLat = -90;
    let minLon = 180;
    let maxLon = -180;
    for (const p of points) {
        if (p.lat < minLat) minLat = p.lat;
        if (p.lat > maxLat) maxLat = p.lat;
        if (p.lon < minLon) minLon = p.lon;
        if (p.lon > maxLon) maxLon = p.lon;
    }
    const cLat = (minLat + maxLat) / 2;
    const cLon = (minLon + maxLon) / 2;
    const spanLat = Math.max(maxLat - minLat, 1);
    const spanLon = Math.max(maxLon - minLon, 1);
    // Equirectangular px-per-degree: lon → 720/360 = 2, lat → 360/180 = 2.
    // Both axes are 2 px/° at zoom 1; the binding span dictates the zoom.
    const zoomLat = (180 * 0.8) / spanLat;
    const zoomLon = (360 * 0.8) / spanLon;
    // Cap auto-fit zoom at 8× — beyond that the user can't visually parse
    // a full region, and clusters will overlap the controls. Manual `+`
    // pushes past this if they want to dial in further.
    const zoom = clampZoom(Math.min(zoomLat, zoomLon, 8));
    return {
        cx: (cLon / 360) * W,
        cy: -(cLat / 180) * H,
        zoom,
    };
}

function clampZoom(z: number): number {
    return Math.max(1, Math.min(64, z));
}

// Anchored zoom — the cursor stays put while the world scales around it.
function zoomAt(
    v: Viewport,
    el: SVGSVGElement,
    clientX: number,
    clientY: number,
    factor: number,
): Viewport {
    const newZoom = clampZoom(v.zoom * factor);
    const rect = el.getBoundingClientRect();
    // Cursor as fraction of viewport (-0.5..0.5).
    const fx = (clientX - rect.left) / rect.width - 0.5;
    const fy = (clientY - rect.top) / rect.height - 0.5;
    // Shift cx/cy so the cursor's world coord stays under it.
    const k = 1 - v.zoom / newZoom;
    return {
        cx: v.cx + fx * (W / v.zoom) * k,
        cy: v.cy + fy * (H / v.zoom) * k,
        zoom: newZoom,
    };
}

// Cluster radius — kept ~constant in *screen pixels* regardless of how
// far the user has zoomed in. We compute the desired pixel radius from
// the photo count, then divide by the zoom factor to convert into the
// SVG's projection units (which shrink as we zoom). Without this scaling
// the bubble appears tiny at world view (zoom=1) and fills the canvas
// at zoom=30 (auto-fit on a regional library).
function clusterRadius(count: number, zoom: number): number {
    // Base radius in screen px: 16 px for 1 photo, ~40 px for 1000.
    const screenPx = 14 + Math.log10(count + 1) * 6;
    return screenPx / zoom;
}

// Two clusters merge when their centroids are closer than the sum of
// their projected radii — prevents overlap halos at low zoom levels.
function mergeOverlapping(raw: ClusterRaw[], zoom: number): ClusterRaw[] {
    const out: ClusterRaw[] = [];
    for (const c of raw) {
        const cx = W / 2 + (c.avgLon / 360) * W;
        const cy = H / 2 - (c.avgLat / 180) * H;
        const cr = clusterRadius(c.points.length, zoom) * 1.6;
        let merged = false;
        for (const o of out) {
            const ox = W / 2 + (o.avgLon / 360) * W;
            const oy = H / 2 - (o.avgLat / 180) * H;
            const or = clusterRadius(o.points.length, zoom) * 1.6;
            const dx = ox - cx;
            const dy = oy - cy;
            if (Math.sqrt(dx * dx + dy * dy) < cr + or) {
                // Re-centroid weighted by counts.
                const total = o.points.length + c.points.length;
                o.avgLat =
                    (o.avgLat * o.points.length + c.avgLat * c.points.length) / total;
                o.avgLon =
                    (o.avgLon * o.points.length + c.avgLon * c.points.length) / total;
                o.points = o.points.concat(c.points);
                merged = true;
                break;
            }
        }
        if (!merged) out.push(c);
    }
    return out;
}
