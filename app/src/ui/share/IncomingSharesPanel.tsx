import { useCallback, useEffect, useState } from "react";
import { api } from "../../ipc";
import type { IncomingShareView } from "../../bindings/IncomingShareView";

/**
 * Pending-invites section on the Peers page. Uses plain useState +
 * interval polling (matches the rest of Peers.tsx) rather than React
 * Query so refresh cadence is visible and explicit.
 */
export default function IncomingSharesPanel() {
    const [incoming, setIncoming] = useState<IncomingShareView[]>([]);
    const [paste, setPaste] = useState("");
    const [msg, setMsg] = useState<string | null>(null);
    const [err, setErr] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    const refresh = useCallback(async () => {
        try {
            setIncoming(await api.listIncomingShares());
        } catch (e) {
            console.warn("listIncomingShares failed", e);
        }
    }, []);

    useEffect(() => {
        void refresh();
        const h = window.setInterval(() => {
            void refresh();
        }, 10_000);
        return () => window.clearInterval(h);
    }, [refresh]);

    const acceptPaste = async () => {
        setBusy(true);
        setErr(null);
        setMsg(null);
        try {
            const cid = await api.acceptIncomingShare(paste.trim());
            setMsg(`accepted invite → collection #${cid}`);
            setPaste("");
            await refresh();
        } catch (e) {
            setErr(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <section className="incoming-shares-panel">
            <h3>Pending shares</h3>
            <p className="incoming-intro">
                Paste a namespace ticket another peer sent you to join a
                shared album.
            </p>
            <textarea
                rows={4}
                value={paste}
                onChange={(e) => setPaste(e.target.value)}
                disabled={busy}
                placeholder="namespace ticket base32…"
            />
            <button onClick={acceptPaste} disabled={busy || !paste.trim()}>
                {busy ? "Accepting…" : "Accept invite"}
            </button>
            {err && <p className="error">{err}</p>}
            {msg && <p className="success">{msg}</p>}

            {incoming.length > 0 && (
                <>
                    <h4>Tracked shares</h4>
                    <ul className="incoming-list">
                        {incoming.map((inc) => (
                            <li key={inc.namespace_id_hex}>
                                <div className="incoming-row">
                                    <code>
                                        {inc.namespace_id_hex.slice(0, 16)}…
                                    </code>
                                    <span className={`state state-${inc.state}`}>
                                        {inc.state}
                                    </span>
                                    <span className="name">
                                        {inc.album_name ?? "(pending name)"}
                                    </span>
                                </div>
                            </li>
                        ))}
                    </ul>
                </>
            )}
        </section>
    );
}
