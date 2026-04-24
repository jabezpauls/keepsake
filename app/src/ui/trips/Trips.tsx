import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../ipc";
import type { TripView } from "../../bindings/TripView";
import { useSession } from "../../state/session";

export default function Trips() {
    const setView = useSession((s) => s.setView);
    const queryClient = useQueryClient();
    const trips = useQuery<TripView[]>({
        queryKey: ["trips"],
        queryFn: api.listTrips,
    });
    const detect = useMutation({
        mutationFn: api.detectTripsRun,
        onSuccess: () => queryClient.invalidateQueries({ queryKey: ["trips"] }),
    });

    return (
        <div className="trips-view">
            <nav className="trips-nav">
                <h2>Trips</h2>
                <span className="spacer" />
                <button
                    onClick={() => detect.mutate()}
                    disabled={detect.isPending}
                >
                    {detect.isPending ? "Detecting…" : "Re-detect"}
                </button>
            </nav>
            {detect.isError && (
                <p className="error">{String(detect.error)}</p>
            )}
            {detect.data != null && (
                <p className="muted">Detected {detect.data} trip(s).</p>
            )}
            {trips.isLoading && <p>Loading…</p>}
            {trips.data && trips.data.length === 0 && (
                <p className="muted">
                    No trips yet. Make sure some of your photos have GPS
                    metadata, then click “Re-detect”.
                </p>
            )}
            <ul className="trip-list">
                {(trips.data ?? []).map((t) => (
                    <li key={t.id}>
                        <button
                            className="trip-card"
                            onClick={() =>
                                setView({
                                    kind: "album",
                                    id: t.id,
                                    name: t.name,
                                })
                            }
                        >
                            <div className="trip-name">{t.name}</div>
                            <div className="trip-meta">
                                {String(t.member_count)} photos
                            </div>
                        </button>
                    </li>
                ))}
            </ul>
        </div>
    );
}
