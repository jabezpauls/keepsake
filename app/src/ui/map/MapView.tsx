import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import ThumbImage from "../timeline/ThumbImage";
import type { MapPoint } from "../../bindings/MapPoint";

// Minimal offline map: equirectangular scatter plot in SVG. Phase-2 tile
// integration is deferred until PMTiles sources are decided; this keeps the
// app fully offline and dependency-light while still surfacing geo data.
export default function MapView() {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const query = useQuery({
        queryKey: ["map-points"],
        queryFn: () => api.mapPoints(null, null, null),
    });

    const [view, setViewport] = useState({ cx: 0, cy: 0, zoom: 1 });
    const [selected, setSelected] = useState<MapPoint[] | null>(null);
    const svgRef = useRef<SVGSVGElement | null>(null);

    const points = useMemo(() => query.data ?? [], [query.data]);

    // Cluster points by grid cell for current zoom.
    const clusters = useMemo(() => {
        if (!points.length) return [];
        const gridSize = 12 / Math.max(view.zoom, 1); // degrees per cell
        const buckets = new Map<string, MapPoint[]>();
        for (const p of points) {
            const gx = Math.floor(p.lon / gridSize);
            const gy = Math.floor(p.lat / gridSize);
            const key = `${gx}:${gy}`;
            const arr = buckets.get(key) ?? [];
            arr.push(p);
            buckets.set(key, arr);
        }
        return Array.from(buckets.values());
    }, [points, view.zoom]);

    useEffect(() => {
        if (!svgRef.current) return;
        const el = svgRef.current;
        const onWheel = (e: WheelEvent) => {
            e.preventDefault();
            setViewport((v) => ({
                ...v,
                zoom: Math.max(1, Math.min(16, v.zoom * (e.deltaY < 0 ? 1.2 : 1 / 1.2))),
            }));
        };
        el.addEventListener("wheel", onWheel, { passive: false });
        return () => el.removeEventListener("wheel", onWheel);
    }, []);

    // Equirectangular projection → SVG viewBox. We use 720×360 so one unit ≈
    // 0.5 degrees. cx/cy is the center offset in the same units.
    const W = 720;
    const H = 360;
    const project = (lat: number, lon: number) => {
        const x = W / 2 + (lon / 360) * W - view.cx;
        const y = H / 2 - (lat / 180) * H - view.cy;
        return { x, y };
    };
    const boxW = W / view.zoom;
    const boxH = H / view.zoom;

    if (query.isLoading) return <div className="timeline-loading">Loading map…</div>;
    if (query.isError) return <div className="timeline-error">Map failed.</div>;
    if (points.length === 0) {
        return (
            <div className="timeline-empty">
                <p>No geo-tagged assets yet.</p>
            </div>
        );
    }

    return (
        <div className="map-view">
            <div className="map-header">
                <span>
                    {points.length} geo-tagged • zoom {view.zoom.toFixed(1)}×
                </span>
                <button onClick={() => setViewport({ cx: 0, cy: 0, zoom: 1 })}>Reset</button>
            </div>
            <svg
                ref={svgRef}
                className="map-canvas"
                viewBox={`${W / 2 - boxW / 2 + view.cx} ${H / 2 - boxH / 2 + view.cy} ${boxW} ${boxH}`}
                preserveAspectRatio="xMidYMid meet"
            >
                <rect x={0} y={0} width={W} height={H} fill="#121822" />
                {/* Latitude gridlines every 30° */}
                {[-60, -30, 0, 30, 60].map((lat) => {
                    const { y } = project(lat, 0);
                    return (
                        <line
                            key={`lat-${lat}`}
                            x1={0}
                            y1={y}
                            x2={W}
                            y2={y}
                            stroke="#283044"
                            strokeWidth={0.5 / view.zoom}
                        />
                    );
                })}
                {[-120, -60, 0, 60, 120].map((lon) => {
                    const { x } = project(0, lon);
                    return (
                        <line
                            key={`lon-${lon}`}
                            x1={x}
                            y1={0}
                            x2={x}
                            y2={H}
                            stroke="#283044"
                            strokeWidth={0.5 / view.zoom}
                        />
                    );
                })}
                {clusters.map((cluster, i) => {
                    const avgLat =
                        cluster.reduce((s, p) => s + p.lat, 0) / cluster.length;
                    const avgLon =
                        cluster.reduce((s, p) => s + p.lon, 0) / cluster.length;
                    const { x, y } = project(avgLat, avgLon);
                    const r = Math.min(10, 3 + Math.log(cluster.length + 1) * 2);
                    return (
                        <g key={i} onClick={() => setSelected(cluster)} style={{ cursor: "pointer" }}>
                            <circle cx={x} cy={y} r={r / view.zoom} fill="#4a9eff" opacity={0.7} />
                            {cluster.length > 1 && (
                                <text
                                    x={x}
                                    y={y + 1 / view.zoom}
                                    textAnchor="middle"
                                    fill="#fff"
                                    fontSize={4 / view.zoom}
                                    fontWeight="bold"
                                >
                                    {cluster.length}
                                </text>
                            )}
                        </g>
                    );
                })}
            </svg>
            {selected && (
                <div className="map-sheet">
                    <div className="map-sheet-header">
                        <span>{selected.length} photos</span>
                        <button onClick={() => setSelected(null)}>Close</button>
                    </div>
                    <div className="map-sheet-grid">
                        {selected.slice(0, 60).map((p) => (
                            <button
                                key={p.asset_id}
                                className="timeline-cell"
                                onClick={() =>
                                    setView({ kind: "asset", id: p.asset_id, back: currentView })
                                }
                            >
                                <ThumbImage assetId={p.asset_id} size={256} mime="image/jpeg" alt="" />
                            </button>
                        ))}
                    </div>
                </div>
            )}
        </div>
    );
}
