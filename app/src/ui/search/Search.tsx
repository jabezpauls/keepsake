import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../ipc";
import { useSession } from "../../state/session";
import ThumbImage from "../timeline/ThumbImage";
import type { SearchRequest } from "../../bindings/SearchRequest";

type ToggleKey = "has_faces" | "is_video" | "is_raw" | "is_screenshot" | "is_live";

export default function Search() {
    const setView = useSession((s) => s.setView);
    const currentView = useSession((s) => s.view);

    const [text, setText] = useState("");
    const [exactText, setExactText] = useState("");
    const [exactResults, setExactResults] = useState<
        import("../../ipc").SearchHitView[] | null
    >(null);
    const [exactBusy, setExactBusy] = useState(false);
    const [camera, setCamera] = useState("");
    const [lens, setLens] = useState("");
    const [toggles, setToggles] = useState<Record<ToggleKey, boolean | undefined>>({
        has_faces: undefined,
        is_video: undefined,
        is_raw: undefined,
        is_screenshot: undefined,
        is_live: undefined,
    });
    const [request, setRequest] = useState<SearchRequest | null>(null);

    const query = useQuery({
        queryKey: ["search", request],
        queryFn: () => (request ? api.searchAssets(request) : Promise.resolve([])),
        enabled: !!request,
    });

    const runExact = async () => {
        setExactBusy(true);
        try {
            const hits = await api.searchTextExact(exactText, 200);
            setExactResults(hits);
            setRequest(null);
        } finally {
            setExactBusy(false);
        }
    };

    const run = () => {
        setExactResults(null);
        const req: SearchRequest = {
            text: text.trim() || null,
            person_ids: [],
            after_day: null,
            before_day: null,
            source_id: null,
            has_faces: toggles.has_faces ?? null,
            is_video: toggles.is_video ?? null,
            is_raw: toggles.is_raw ?? null,
            is_screenshot: toggles.is_screenshot ?? null,
            is_live: toggles.is_live ?? null,
            camera_make: camera.trim() || null,
            lens: lens.trim() || null,
            limit: 100,
        };
        setRequest(req);
    };

    const cycleToggle = (key: ToggleKey) => {
        setToggles((t) => {
            const cur = t[key];
            const next = cur === undefined ? true : cur === true ? false : undefined;
            return { ...t, [key]: next };
        });
    };

    const chipClass = (v: boolean | undefined) =>
        v === undefined ? "chip" : v ? "chip on" : "chip off";

    return (
        <div className="search-view">
            <form
                className="search-bar"
                onSubmit={(e) => {
                    e.preventDefault();
                    run();
                }}
            >
                <input
                    autoFocus
                    placeholder="Search — natural language, EXIF text, camera model…"
                    value={text}
                    onChange={(e) => setText(e.target.value)}
                />
                <button type="submit">Search</button>
            </form>
            <div className="search-chips">
                <button type="button" className={chipClass(toggles.has_faces)} onClick={() => cycleToggle("has_faces")}>
                    {toggles.has_faces === false ? "No faces" : "Faces"}
                </button>
                <button type="button" className={chipClass(toggles.is_video)} onClick={() => cycleToggle("is_video")}>
                    {toggles.is_video === false ? "Not video" : "Video"}
                </button>
                <button type="button" className={chipClass(toggles.is_raw)} onClick={() => cycleToggle("is_raw")}>
                    RAW
                </button>
                <button
                    type="button"
                    className={chipClass(toggles.is_screenshot)}
                    onClick={() => cycleToggle("is_screenshot")}
                >
                    Screenshots
                </button>
                <button type="button" className={chipClass(toggles.is_live)} onClick={() => cycleToggle("is_live")}>
                    Live
                </button>
                <input
                    className="search-text-chip"
                    placeholder="Camera"
                    value={camera}
                    onChange={(e) => setCamera(e.target.value)}
                />
                <input
                    className="search-text-chip"
                    placeholder="Lens"
                    value={lens}
                    onChange={(e) => setLens(e.target.value)}
                />
            </div>
            <form
                className="search-bar"
                onSubmit={(e) => {
                    e.preventDefault();
                    if (exactText.trim()) void runExact();
                }}
            >
                <input
                    placeholder="Exact text (whole words, OCR + captions)"
                    value={exactText}
                    onChange={(e) => setExactText(e.target.value)}
                    disabled={exactBusy}
                />
                <button type="submit" disabled={!exactText.trim() || exactBusy}>
                    {exactBusy ? "Searching…" : "Exact"}
                </button>
            </form>

            <div className="search-results">
                {exactResults !== null && (
                    <>
                        {exactResults.length === 0 && (
                            <div className="timeline-empty">No exact-text matches.</div>
                        )}
                        <div className="timeline-row wrap">
                            {exactResults.map((hit, idx) => (
                                <button
                                    key={hit.id}
                                    className="timeline-cell"
                                    onClick={() =>
                                        setView({
                                            kind: "asset",
                                            id: hit.id,
                                            back: currentView,
                                            neighbors: exactResults.map((h) => h.id),
                                            index: idx,
                                        })
                                    }
                                >
                                    <ThumbImage assetId={hit.id} size={256} mime={hit.mime} alt="" />
                                    {hit.is_video && <span className="cell-badge">▶</span>}
                                    {hit.is_live && <span className="cell-badge">LIVE</span>}
                                </button>
                            ))}
                        </div>
                    </>
                )}
                {exactResults === null && query.isLoading && (
                    <div className="timeline-loading">Searching…</div>
                )}
                {exactResults === null && query.data?.length === 0 && (
                    <div className="timeline-empty">No matches.</div>
                )}
                {exactResults === null && (
                    <div className="timeline-row wrap">
                        {query.data?.map((hit, idx) => (
                            <button
                                key={hit.id}
                                className="timeline-cell"
                                onClick={() =>
                                    setView({
                                        kind: "asset",
                                        id: hit.id,
                                        back: currentView,
                                        neighbors: query.data!.map((h) => h.id),
                                        index: idx,
                                    })
                                }
                            >
                                <ThumbImage assetId={hit.id} size={256} mime={hit.mime} alt="" />
                                {hit.is_video && <span className="cell-badge">▶</span>}
                                {hit.is_live && <span className="cell-badge">LIVE</span>}
                                {hit.score !== null && (
                                    <span className="cell-score">{hit.score.toFixed(2)}</span>
                                )}
                            </button>
                        ))}
                    </div>
                )}
            </div>
        </div>
    );
}
