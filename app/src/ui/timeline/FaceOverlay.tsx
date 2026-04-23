import { useQuery } from "@tanstack/react-query";
import { api } from "../../ipc";

interface Props {
    assetId: number;
    imgWidth: number;   // thumb1024 natural width (from <img>.naturalWidth)
    imgHeight: number;  // thumb1024 natural height
    visible: boolean;
    onPersonClick: (personId: number, personName: string | null) => void;
}

/**
 * Circular face rings overlaid on the photo viewer (Apple-Photos style).
 * Rings are positioned by % — bbox.x/imgWidth × 100 — so they scale with
 * whatever size the <img> is rendered at. Named chips navigate to the
 * person's grid; unnamed faces render a neutral ring without a chip.
 */
export default function FaceOverlay({
    assetId,
    imgWidth,
    imgHeight,
    visible,
    onPersonClick,
}: Props) {
    const query = useQuery({
        queryKey: ["asset-faces", assetId],
        queryFn: () => api.assetFaces(assetId),
        staleTime: 60_000,
    });

    if (!visible || !imgWidth || !imgHeight) return null;
    if (query.isLoading || !query.data || query.data.length === 0) return null;

    return (
        <div className="face-overlay-layer" aria-hidden={!visible}>
            {query.data.map((f) => {
                const [x, y, w, h] = f.bbox;
                const leftPct = (x / imgWidth) * 100;
                const topPct = (y / imgHeight) * 100;
                const widthPct = (w / imgWidth) * 100;
                const heightPct = (h / imgHeight) * 100;
                const clickable = f.person_id !== null;
                return (
                    <button
                        key={f.face_id}
                        className={`face-overlay-ring${clickable ? " clickable" : " unassigned"}`}
                        style={{
                            left: `${leftPct}%`,
                            top: `${topPct}%`,
                            width: `${widthPct}%`,
                            height: `${heightPct}%`,
                        }}
                        onClick={
                            clickable
                                ? () => onPersonClick(f.person_id!, f.person_name ?? null)
                                : undefined
                        }
                        disabled={!clickable}
                        aria-label={
                            f.person_name
                                ? `View photos of ${f.person_name}`
                                : "Unnamed face"
                        }
                        title={f.person_name ?? "Unnamed"}
                    >
                        <span className="face-name-chip">
                            {f.person_name ?? "Unnamed"}
                        </span>
                    </button>
                );
            })}
        </div>
    );
}
