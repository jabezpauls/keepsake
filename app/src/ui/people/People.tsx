import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import PersonFaceThumb from "./PersonFaceThumb";
import type { PersonView } from "../../bindings/PersonView";

export default function People() {
    const qc = useQueryClient();
    const setView = useSession((s) => s.setView);
    const query = useQuery({
        queryKey: ["people"],
        queryFn: () => api.listPeople(false),
    });
    const rename = useMutation({
        mutationFn: ({ id, name }: { id: number; name: string }) =>
            api.renamePerson(id, name),
        onSuccess: () => qc.invalidateQueries({ queryKey: ["people"] }),
    });
    const hide = useMutation({
        mutationFn: ({ id, hidden }: { id: number; hidden: boolean }) =>
            api.hidePerson(id, hidden),
        onSuccess: () => qc.invalidateQueries({ queryKey: ["people"] }),
    });

    const [editing, setEditing] = useState<number | null>(null);
    const [draft, setDraft] = useState("");

    if (query.isLoading) return <div className="timeline-loading">Loading people…</div>;
    if (query.data?.length === 0) {
        return (
            <div className="timeline-empty">
                <p>No people detected yet.</p>
                <p className="muted">
                    Face detection runs when the on-device ML models are installed. Run
                    <code> scripts/download_models.sh</code> and rebuild with
                    <code> --features ml-models</code> to enable.
                </p>
            </div>
        );
    }
    return (
        <div className="people-view">
            <h2>People</h2>
            <div className="people-grid">
                {query.data?.map((p: PersonView) => (
                    <div key={p.id} className="person-card">
                        <button
                            className="person-cover-button"
                            onClick={() =>
                                setView({ kind: "person", id: p.id, name: p.name })
                            }
                            title="View this person's photos"
                        >
                            {p.cover_asset_id !== null ? (
                                <PersonFaceThumb
                                    personId={p.id}
                                    size={256}
                                    alt={p.name ?? "unnamed"}
                                />
                            ) : (
                                <div className="person-cover-empty" />
                            )}
                        </button>
                        <div className="person-body">
                            {editing === p.id ? (
                                <form
                                    onSubmit={(e) => {
                                        e.preventDefault();
                                        rename.mutate({ id: p.id, name: draft });
                                        setEditing(null);
                                    }}
                                >
                                    <input
                                        autoFocus
                                        value={draft}
                                        onChange={(e) => setDraft(e.target.value)}
                                        onBlur={() => setEditing(null)}
                                    />
                                </form>
                            ) : (
                                <button
                                    className="person-name"
                                    onClick={() => {
                                        setEditing(p.id);
                                        setDraft(p.name ?? "");
                                    }}
                                >
                                    {p.name ?? "Add name"}
                                </button>
                            )}
                            <span className="person-count">{p.face_count} faces</span>
                            <button
                                className="muted-button"
                                onClick={() => hide.mutate({ id: p.id, hidden: !p.hidden })}
                            >
                                {p.hidden ? "Unhide" : "Hide"}
                            </button>
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
}
