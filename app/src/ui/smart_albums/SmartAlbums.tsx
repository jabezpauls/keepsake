import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../ipc";
import type { SmartAlbumView } from "../../bindings/SmartAlbumView";
import type { SmartRuleView } from "../../bindings/SmartRuleView";
import { useSession } from "../../state/session";

const EMPTY_RULE: SmartRuleView = {
    is_raw: null,
    is_video: null,
    is_screenshot: null,
    is_live: null,
    has_faces: null,
    camera_make: null,
    lens: null,
    source_id: null,
    person_ids: [],
    after_day: null,
    before_day: null,
};

function ruleIsEmpty(r: SmartRuleView): boolean {
    return (
        r.is_raw === null &&
        r.is_video === null &&
        r.is_screenshot === null &&
        r.is_live === null &&
        r.has_faces === null &&
        !r.camera_make &&
        !r.lens &&
        r.source_id === null &&
        r.person_ids.length === 0 &&
        r.after_day === null &&
        r.before_day === null
    );
}

function describeRule(r: SmartRuleView): string {
    const parts: string[] = [];
    const flag = (k: keyof SmartRuleView, onLabel: string, offLabel: string) => {
        const v = r[k];
        if (v === true) parts.push(onLabel);
        else if (v === false) parts.push(offLabel);
    };
    flag("is_raw", "RAW", "not RAW");
    flag("is_video", "video", "not video");
    flag("is_screenshot", "screenshot", "not screenshot");
    flag("is_live", "live", "not live");
    flag("has_faces", "has faces", "no faces");
    if (r.camera_make) parts.push(`camera "${r.camera_make}"`);
    if (r.lens) parts.push(`lens "${r.lens}"`);
    return parts.join(" · ") || "no predicates";
}

export default function SmartAlbums() {
    const setView = useSession((s) => s.setView);
    const queryClient = useQueryClient();
    const albums = useQuery<SmartAlbumView[]>({
        queryKey: ["smart-albums"],
        queryFn: api.listSmartAlbums,
    });

    const [showBuilder, setShowBuilder] = useState(false);

    const refresh = async (id: number) => {
        await api.refreshSmartAlbum(id);
        await queryClient.invalidateQueries({ queryKey: ["smart-albums"] });
        await queryClient.invalidateQueries({ queryKey: ["smart-album-page", id] });
    };

    const remove = async (a: SmartAlbumView) => {
        if (!window.confirm(`Delete smart album "${a.name}"? Underlying photos are kept.`)) return;
        await api.deleteSmartAlbum(a.id);
        await queryClient.invalidateQueries({ queryKey: ["smart-albums"] });
    };

    return (
        <div className="smart-albums">
            <nav className="smart-albums-nav">
                <h2>Smart Albums</h2>
                <span className="spacer" />
                <button onClick={() => setShowBuilder((s) => !s)}>
                    {showBuilder ? "Cancel" : "+ New smart album"}
                </button>
            </nav>

            {showBuilder && (
                <SmartAlbumBuilder
                    onSaved={async () => {
                        setShowBuilder(false);
                        await queryClient.invalidateQueries({ queryKey: ["smart-albums"] });
                    }}
                />
            )}

            {albums.isLoading && <p>Loading…</p>}
            {albums.data?.length === 0 && !showBuilder && (
                <p className="muted">
                    No smart albums yet. Click <strong>+ New smart album</strong> to build one
                    from a rule (e.g., "all RAW photos from SONY").
                </p>
            )}

            <ul className="smart-album-list">
                {albums.data?.map((a) => (
                    <li key={a.id} className="smart-album-row">
                        <button
                            className="smart-album-open"
                            onClick={() => setView({ kind: "smart_album", id: a.id, name: a.name })}
                        >
                            <strong>{a.name}</strong>
                            <span className="smart-album-rule">{describeRule(a.rule)}</span>
                            <span className="count">{a.member_count} items</span>
                            {a.snapshot_at !== null && (
                                <span className="muted">
                                    · refreshed {new Date(a.snapshot_at * 1000).toLocaleString()}
                                </span>
                            )}
                        </button>
                        <button onClick={() => refresh(a.id)} title="Rematerialise">
                            Refresh
                        </button>
                        <button className="danger" onClick={() => remove(a)} title="Delete">
                            Delete
                        </button>
                    </li>
                ))}
            </ul>
        </div>
    );
}

function SmartAlbumBuilder({ onSaved }: { onSaved: () => Promise<void> }) {
    const [name, setName] = useState("");
    const [rule, setRule] = useState<SmartRuleView>(EMPTY_RULE);
    const [camera, setCamera] = useState("");
    const [lens, setLens] = useState("");
    const [busy, setBusy] = useState(false);
    const [err, setErr] = useState<string | null>(null);

    type BoolFlag = "is_raw" | "is_video" | "is_screenshot" | "is_live" | "has_faces";

    const cycle = (k: BoolFlag) => {
        setRule((r) => {
            const cur = r[k];
            const next = cur === null ? true : cur === true ? false : null;
            return { ...r, [k]: next };
        });
    };

    const chipClass = (v: boolean | null) =>
        v === null ? "chip" : v ? "chip on" : "chip off";

    const label = (k: BoolFlag, onText: string, neutralText?: string) => {
        const v = rule[k];
        if (v === false) return `Not ${onText.toLowerCase()}`;
        if (v === true) return onText;
        return neutralText ?? onText;
    };

    const save = async () => {
        setErr(null);
        if (!name.trim()) {
            setErr("Name is required.");
            return;
        }
        const finalRule: SmartRuleView = {
            ...rule,
            camera_make: camera.trim() || null,
            lens: lens.trim() || null,
        };
        if (ruleIsEmpty(finalRule)) {
            setErr("Pick at least one predicate — an empty rule matches nothing.");
            return;
        }
        setBusy(true);
        try {
            await api.createSmartAlbum(name, finalRule);
            await onSaved();
        } catch (e) {
            setErr(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="smart-album-builder">
            <input
                className="smart-album-name"
                placeholder="Name — e.g., “SONY RAW 2024”"
                value={name}
                onChange={(e) => setName(e.target.value)}
                disabled={busy}
            />
            <div className="search-chips">
                <button type="button" className={chipClass(rule.is_raw)} onClick={() => cycle("is_raw")}>
                    {label("is_raw", "RAW")}
                </button>
                <button type="button" className={chipClass(rule.is_video)} onClick={() => cycle("is_video")}>
                    {label("is_video", "Video")}
                </button>
                <button
                    type="button"
                    className={chipClass(rule.is_screenshot)}
                    onClick={() => cycle("is_screenshot")}
                >
                    {label("is_screenshot", "Screenshot")}
                </button>
                <button type="button" className={chipClass(rule.is_live)} onClick={() => cycle("is_live")}>
                    {label("is_live", "Live")}
                </button>
                <button
                    type="button"
                    className={chipClass(rule.has_faces)}
                    onClick={() => cycle("has_faces")}
                >
                    {label("has_faces", "Has faces", "Faces")}
                </button>
                <input
                    className="search-text-chip"
                    placeholder="Camera"
                    value={camera}
                    onChange={(e) => setCamera(e.target.value)}
                    disabled={busy}
                />
                <input
                    className="search-text-chip"
                    placeholder="Lens"
                    value={lens}
                    onChange={(e) => setLens(e.target.value)}
                    disabled={busy}
                />
            </div>
            {err && <p className="error">{err}</p>}
            <div className="smart-album-builder-actions">
                <button onClick={save} disabled={busy}>
                    {busy ? "Materialising…" : "Save & materialise"}
                </button>
            </div>
        </div>
    );
}
