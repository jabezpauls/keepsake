import {
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
} from "react";
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

    // Tracking the SVG's actual rendered size so HTML markers overlaid on
    // top can re-project to canvas pixels through pan/zoom changes.
    const [canvasSize, setCanvasSize] = useState({ w: 0, h: 0 });
    useLayoutEffect(() => {
        const el = svgRef.current;
        if (!el) return;
        const resize = () => {
            const r = el.getBoundingClientRect();
            setCanvasSize({ w: r.width, h: r.height });
        };
        resize();
        const ro = new ResizeObserver(resize);
        ro.observe(el);
        return () => ro.disconnect();
    }, [query.data]);

    const points = useMemo(() => query.data ?? [], [query.data]);

    // First-load auto-fit: compute the lat/lon extent of every point and
    // set the viewport so the bounding box fills 80% of the canvas. Only
    // runs once per mount — manual pan/zoom afterwards is sticky.
    useEffect(() => {
        if (fitOnceRef.current || points.length === 0) return;
        fitOnceRef.current = true;
        setViewport(fitToPoints(points));
    }, [points]);

    // Cluster: grid-bucket, then merge clusters that visually overlap on
    // the rendered canvas. The merge step uses the *actual* canvas size
    // so the threshold matches what the user sees — earlier versions
    // computed the test in viewBox units, which under-estimated overlap
    // by ~1.7× and produced one giant blob at high zoom even when the
    // bucket grid had already split landmarks apart.
    //
    // Bucket-size scales as `2/zoom` (much finer than the original
    // `12/zoom`): adjacent landmarks 0.05° apart split into different
    // buckets by zoom 40. Photos within a single landmark (≤0.003° σ)
    // stay merged at every zoom — they belong in one circle.
    const clusters = useMemo(() => {
        if (!points.length) return [];
        const gridDeg = 2 / Math.max(viewport.zoom, 1);
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
        return mergeOverlapping(raw, viewport.zoom, canvasSize.w);
    }, [points, viewport.zoom, canvasSize.w]);

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
            <div className="kp-map-stage">
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

            </svg>
            {/* Snap-Map-style HTML markers overlaid on the SVG. Sized in
              * screen pixels (constant regardless of zoom) and positioned
              * via viewBox→canvas re-projection so they stay glued to the
              * map through pan + zoom. */}
            <div className="kp-map-markers" aria-hidden={selected ? "true" : undefined}>
                {clusters.map((c, i) => {
                    const screen = projectToCanvas(
                        c.avgLat,
                        c.avgLon,
                        viewport,
                        canvasSize,
                    );
                    if (!screen) return null;
                    return (
                        <ClusterMarker
                            key={i}
                            cluster={c}
                            zoom={viewport.zoom}
                            x={screen.x}
                            y={screen.y}
                            onSelect={() => setSelected(c.points)}
                        />
                    );
                })}
            </div>
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

interface ClusterRaw {
    points: MapPoint[];
    avgLat: number;
    avgLon: number;
}

// Compute the viewport that fits the *dense* region of points with 10%
// margin. Uses the 5th–95th percentile of lat/lon instead of raw min/max
// so a handful of far-flung outliers (e.g. an old phone backup with one
// trip to Tokyo amid 600 photos in Bangalore) doesn't pull the fit out
// to a global bbox centred over open ocean. Outliers stay visible — they
// just don't dictate the initial frame.
function fitToPoints(points: MapPoint[]): Viewport {
    if (points.length === 0) return INITIAL_VIEWPORT;
    const lats = points.map((p) => p.lat).sort((a, b) => a - b);
    const lons = points.map((p) => p.lon).sort((a, b) => a - b);
    const lo = (xs: number[]) => xs[Math.floor(xs.length * 0.05)] ?? xs[0];
    const hi = (xs: number[]) => xs[Math.floor(xs.length * 0.95)] ?? xs[xs.length - 1];
    const minLat = lo(lats);
    const maxLat = hi(lats);
    const minLon = lo(lons);
    const maxLon = hi(lons);
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

// Cluster bucketing keeps subdividing as zoom rises (gridDeg = 12/zoom),
// so even without tile detail higher zoom is useful: a 600-photo cluster
// at zoom 8 splits into per-neighborhood markers by ~64×, then individual
// pin-points by ~128×. Floor stays at 1× (full world).
function clampZoom(z: number): number {
    return Math.max(1, Math.min(128, z));
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

// Cluster marker size in screen pixels — constant regardless of zoom.
// Snapchat's Snap Map sticks to ~36 px for individual snaps; we scale
// gently with count so a 600-photo cluster reads as denser without
// dwarfing single-photo markers next to it. At deep zoom (the per-photo
// rendering path), shrink singletons so adjacent pins from a tight
// landmark cluster don't visually collide.
function markerSize(count: number, zoom: number): number {
    if (count === 1) return zoom > 48 ? 18 : 36;
    return Math.min(56, 36 + Math.log10(count) * 8);
}

// Two clusters merge when their centroids are closer than the sum of
// their *screen-pixel* radii — earlier versions did the test in viewBox
// units, which under-estimated overlap by canvasW / W (~1.7× on a
// 1270 px canvas with our 720 px viewBox), so adjacent landmarks at
// deep zoom always re-merged into one bubble after bucketing already
// split them. Now we project to canvas pixels using the same
// `xMidYMid meet` math as `projectToCanvas` and compare directly.
function mergeOverlapping(
    raw: ClusterRaw[],
    zoom: number,
    canvasW: number,
): ClusterRaw[] {
    // Scale: 1 viewBox unit → how many screen pixels (uniform — viewBox
    // and canvas may differ in aspect ratio, but `xMidYMid meet` picks
    // the smaller of the two, which on our common landscape canvas is
    // canvasW / vbW). Falls back to the old behaviour when canvas hasn't
    // measured yet (avoids divide-by-zero on first render).
    const pxPerVb = canvasW > 0 ? (canvasW * zoom) / W : zoom;
    const out: ClusterRaw[] = [];
    for (const c of raw) {
        const cx = W / 2 + (c.avgLon / 360) * W;
        const cy = H / 2 - (c.avgLat / 180) * H;
        const cr = markerSize(c.points.length, zoom) / 2;
        let merged = false;
        for (const o of out) {
            const ox = W / 2 + (o.avgLon / 360) * W;
            const oy = H / 2 - (o.avgLat / 180) * H;
            const or = markerSize(o.points.length, zoom) / 2;
            const dxPx = (ox - cx) * pxPerVb;
            const dyPx = (oy - cy) * pxPerVb;
            if (Math.sqrt(dxPx * dxPx + dyPx * dyPx) < cr + or) {
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

// Project (lat, lon) world coords → canvas pixel coords, accounting for
// the SVG's xMidYMid-meet aspect-ratio fit (which letterboxes one axis
// when the canvas's ratio doesn't match our 2:1 viewBox).
function projectToCanvas(
    lat: number,
    lon: number,
    v: Viewport,
    canvas: { w: number; h: number },
): { x: number; y: number } | null {
    if (canvas.w === 0 || canvas.h === 0) return null;
    const worldX = W / 2 + (lon / 360) * W;
    const worldY = H / 2 - (lat / 180) * H;
    const vbX = W / 2 - W / v.zoom / 2 + v.cx;
    const vbY = H / 2 - H / v.zoom / 2 + v.cy;
    const vbW = W / v.zoom;
    const vbH = H / v.zoom;
    // xMidYMid meet → uniform scale = min, viewBox region centred.
    const scale = Math.min(canvas.w / vbW, canvas.h / vbH);
    const regionW = vbW * scale;
    const regionH = vbH * scale;
    const offsetX = (canvas.w - regionW) / 2;
    const offsetY = (canvas.h - regionH) / 2;
    return {
        x: offsetX + (worldX - vbX) * scale,
        y: offsetY + (worldY - vbY) * scale,
    };
}

interface ClusterMarkerProps {
    cluster: ClusterRaw;
    zoom: number;
    x: number;
    y: number;
    onSelect: () => void;
}

// Snap-Map-style marker: circular thumbnail of a representative photo,
// crisp white ring, drop shadow, and (when the cluster is > 1) a small
// count badge in the lower-right corner. Centred on (x, y).
function ClusterMarker({ cluster, zoom, x, y, onSelect }: ClusterMarkerProps) {
    const size = markerSize(cluster.points.length, zoom);
    // Pick the median point's asset_id as the cover so the same cluster
    // shows the same thumbnail across re-renders (avoids flicker as the
    // grid bucketing shifts a few photos around).
    const cover =
        cluster.points[Math.floor(cluster.points.length / 2)]?.asset_id ??
        cluster.points[0]?.asset_id;
    if (cover == null) return null;
    return (
        <button
            type="button"
            className="kp-map-marker"
            style={{
                left: `${x}px`,
                top: `${y}px`,
                width: `${size}px`,
                height: `${size}px`,
            }}
            onClick={onSelect}
            aria-label={`${cluster.points.length} ${
                cluster.points.length === 1 ? "photo" : "photos"
            } at ${cluster.avgLat.toFixed(2)}, ${cluster.avgLon.toFixed(2)}`}
        >
            <span className="kp-map-marker-photo">
                <ThumbImage assetId={cover} size={128} mime="image/jpeg" alt="" />
            </span>
            {cluster.points.length > 1 && (
                <span className="kp-map-marker-badge">
                    {cluster.points.length > 999 ? "999+" : cluster.points.length}
                </span>
            )}
        </button>
    );
}
