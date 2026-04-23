import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../ipc";
import type { IngestStatus } from "../../bindings/IngestStatus";
import type { SourceView } from "../../bindings/SourceView";

type Adapter = "generic" | "iphone_folder" | "google_takeout";

export default function Sources() {
    const sources = useQuery<SourceView[]>({
        queryKey: ["sources"],
        queryFn: () => api.listSources(),
        refetchInterval: 2000,
    });

    return (
        <div className="sources">
            <h2>Sources</h2>
            <AddSourceForm onAdded={() => void sources.refetch()} />
            {sources.isLoading && <p>Loading…</p>}
            {sources.data && sources.data.length === 0 && <p>(no sources yet)</p>}
            <ul className="source-list">
                {sources.data?.map((s) => <SourceRow key={s.id} source={s} />)}
            </ul>
        </div>
    );
}

function AddSourceForm({ onAdded }: { onAdded: () => void }) {
    const [name, setName] = useState("");
    const [root, setRoot] = useState("");
    const [adapter, setAdapter] = useState<Adapter>("generic");
    const [linkedOnly, setLinkedOnly] = useState(false);
    const [busy, setBusy] = useState(false);
    const [err, setErr] = useState<string | null>(null);

    const pickFolder = async () => {
        try {
            const picked = await openDialog({ directory: true, multiple: false });
            if (typeof picked === "string") setRoot(picked);
        } catch {
            /* user cancelled or dialog plugin missing in test env */
        }
    };

    const submit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim() || !root.trim()) {
            setErr("name and folder are required");
            return;
        }
        setBusy(true);
        setErr(null);
        try {
            await api.addSource({ name, root, adapter, linkedOnly });
            setName("");
            setRoot("");
            onAdded();
        } catch (e2) {
            setErr(String(e2));
        } finally {
            setBusy(false);
        }
    };

    return (
        <form className="add-source" onSubmit={submit}>
            <h3>Add a source</h3>
            <label>
                <span>Name</span>
                <input value={name} onChange={(e) => setName(e.target.value)} disabled={busy} />
            </label>
            <div className="field">
                <label htmlFor="source-folder">Folder</label>
                <div className="folder-row">
                    <input
                        id="source-folder"
                        value={root}
                        onChange={(e) => setRoot(e.target.value)}
                        disabled={busy}
                    />
                    <button type="button" onClick={pickFolder} disabled={busy}>
                        Browse…
                    </button>
                </div>
            </div>
            <label>
                <span>Adapter</span>
                <select
                    value={adapter}
                    onChange={(e) => setAdapter(e.target.value as Adapter)}
                    disabled={busy}
                >
                    <option value="generic">Generic folder</option>
                    <option value="iphone_folder">iPhone (DCIM)</option>
                    <option value="google_takeout">Google Takeout</option>
                </select>
            </label>
            <label className="checkbox">
                <input
                    type="checkbox"
                    checked={linkedOnly}
                    onChange={(e) => setLinkedOnly(e.target.checked)}
                    disabled={busy}
                />
                <span>Link only (don't copy into vault)</span>
            </label>
            <button type="submit" disabled={busy}>
                Add source
            </button>
            {err && <p className="error">{err}</p>}
        </form>
    );
}

function SourceRow({ source }: { source: SourceView }) {
    const status = useQuery<IngestStatus>({
        queryKey: ["ingest", source.id],
        queryFn: () => api.ingestStatus(source.id),
        refetchInterval: 1000,
    });
    const s = status.data?.state;

    return (
        <li className="source-row">
            <div className="source-head">
                <strong>{source.name}</strong>
                <span className="kind">{source.adapter_kind}</span>
            </div>
            <div className="source-path">{source.root_path}</div>
            <div className="source-stats">
                {source.file_count} files · {(source.bytes_total / 1024 / 1024).toFixed(1)} MiB
            </div>
            {s && <IngestStateBadge state={s} />}
        </li>
    );
}

function IngestStateBadge({ state }: { state: IngestStatus["state"] }) {
    switch (state.state) {
        case "idle":
            return <span className="ingest idle">idle</span>;
        case "running": {
            const pct =
                state.files_total > 0 ? (state.files_processed / state.files_total) * 100 : 0;
            return (
                <div className="ingest running">
                    <div className="bar">
                        <div className="fill" style={{ width: `${pct}%` }} />
                    </div>
                    <span>
                        {state.files_processed}/{state.files_total}
                    </span>
                </div>
            );
        }
        case "done":
            return (
                <span className="ingest done">
                    done · {state.inserted} new · {state.deduped} dedupe · {state.errors} errors
                </span>
            );
        case "failed":
            return <span className="ingest failed">failed: {state.message}</span>;
    }
}
