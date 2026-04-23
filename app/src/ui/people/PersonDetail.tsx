import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import ThumbImage from "../timeline/ThumbImage";
import PersonFaceThumb from "./PersonFaceThumb";

export default function PersonDetail({
    id,
    name,
}: {
    id: number;
    name: string | null;
}) {
    const qc = useQueryClient();
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);
    const [editing, setEditing] = useState(false);
    const [draft, setDraft] = useState(name ?? "");

    const hits = useQuery({
        queryKey: ["person-assets", id],
        queryFn: () =>
            api.searchAssets({
                text: null,
                person_ids: [id],
                after_day: null,
                before_day: null,
                source_id: null,
                has_faces: null,
                is_video: null,
                is_raw: null,
                is_screenshot: null,
                is_live: null,
                camera_make: null,
                lens: null,
                limit: 500,
            }),
    });

    const rename = useMutation({
        mutationFn: (next: string) => api.renamePerson(id, next),
        onSuccess: () => {
            qc.invalidateQueries({ queryKey: ["people"] });
            qc.invalidateQueries({ queryKey: ["person-assets", id] });
        },
    });

    return (
        <div className="person-detail">
            <div className="person-detail-header">
                <button
                    className="muted-button"
                    onClick={() => setView({ kind: "people" })}
                >
                    ← Back
                </button>
                <PersonFaceThumb
                    personId={id}
                    size={192}
                    className="person-avatar"
                    alt={name ?? "unnamed"}
                />
                {editing ? (
                    <form
                        onSubmit={(e) => {
                            e.preventDefault();
                            rename.mutate(draft);
                            setEditing(false);
                        }}
                    >
                        <input
                            autoFocus
                            value={draft}
                            onChange={(e) => setDraft(e.target.value)}
                            onBlur={() => setEditing(false)}
                            placeholder="Name"
                        />
                    </form>
                ) : (
                    <button
                        className="person-name-edit"
                        onClick={() => {
                            setEditing(true);
                            setDraft(name ?? "");
                        }}
                    >
                        <h2>{name ?? "Unnamed"}</h2>
                        <span className="muted">click to rename</span>
                    </button>
                )}
                <span className="muted">
                    {hits.data?.length ?? 0} photo{hits.data?.length === 1 ? "" : "s"}
                </span>
            </div>
            <div className="timeline-row wrap">
                {hits.data?.map((h) => (
                    <button
                        key={h.id}
                        className="timeline-cell"
                        onClick={() =>
                            setView({ kind: "asset", id: h.id, back: currentView })
                        }
                    >
                        <ThumbImage assetId={h.id} size={256} mime={h.mime} alt="" />
                        {h.is_video && <span className="cell-badge">▶</span>}
                    </button>
                ))}
            </div>
        </div>
    );
}
