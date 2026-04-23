import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import ThumbImage from "../timeline/ThumbImage";

export default function Duplicates() {
    const qc = useQueryClient();
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);

    const clusters = useQuery({
        queryKey: ["near-dup"],
        queryFn: () => api.nearDupList(),
    });
    const rebuild = useMutation({
        mutationFn: () => api.nearDupRebuild(),
        onSuccess: () => qc.invalidateQueries({ queryKey: ["near-dup"] }),
    });

    return (
        <div className="duplicates-view">
            <div className="duplicates-header">
                <h2>Review duplicates</h2>
                <button
                    onClick={() => rebuild.mutate()}
                    disabled={rebuild.isPending}
                >
                    {rebuild.isPending ? "Scanning…" : "Rescan"}
                </button>
            </div>
            {clusters.isLoading && <div className="timeline-loading">Loading clusters…</div>}
            {clusters.data?.length === 0 && (
                <div className="timeline-empty">
                    <p>No near-duplicate groups detected.</p>
                    <p className="muted">
                        If you just ingested new media, click Rescan to regenerate clusters.
                    </p>
                </div>
            )}
            {clusters.data?.map((cluster) => (
                <div key={cluster.cluster_id} className="ndcluster">
                    <div className="ndcluster-header">
                        <span>Group {cluster.cluster_id + 1}</span>
                        <span>{cluster.members.length} items</span>
                    </div>
                    <div className="ndcluster-row">
                        {cluster.members.map((m, idx) => (
                            <button
                                key={m.asset_id}
                                className={`timeline-cell${m.is_best ? " best-shot" : ""}`}
                                onClick={() =>
                                    setView({
                                        kind: "asset",
                                        id: m.asset_id,
                                        back: currentView,
                                        neighbors: cluster.members.map((x) => x.asset_id),
                                        index: idx,
                                    })
                                }
                            >
                                <ThumbImage
                                    assetId={m.asset_id}
                                    size={256}
                                    mime="image/jpeg"
                                    alt=""
                                />
                                {m.is_best && <span className="cell-badge best">★</span>}
                            </button>
                        ))}
                    </div>
                </div>
            ))}
        </div>
    );
}
