import { useCallback, useEffect, useState } from "react";
import { api } from "../../ipc";
import type { PairingTicketView } from "../../bindings/PairingTicketView";
import type { PeerAcceptedView } from "../../bindings/PeerAcceptedView";

export default function Peers() {
    const [ticket, setTicket] = useState<PairingTicketView | null>(null);
    const [ticketErr, setTicketErr] = useState<string | null>(null);
    const [accepted, setAccepted] = useState<PeerAcceptedView[]>([]);
    const [paste, setPaste] = useState("");
    const [acceptErr, setAcceptErr] = useState<string | null>(null);
    const [acceptMsg, setAcceptMsg] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    const refresh = useCallback(async () => {
        try {
            setAccepted(await api.peerList());
        } catch (e) {
            // non-fatal — the list just stays empty on error
            console.warn("peerList failed", e);
        }
    }, []);

    useEffect(() => {
        void refresh();
    }, [refresh]);

    const generateTicket = async () => {
        setBusy(true);
        setTicketErr(null);
        try {
            setTicket(await api.peerMyTicket());
        } catch (e) {
            setTicketErr(String(e));
            setTicket(null);
        } finally {
            setBusy(false);
        }
    };

    const acceptPaste = async () => {
        setBusy(true);
        setAcceptErr(null);
        setAcceptMsg(null);
        try {
            const v = await api.peerAcceptTicket(paste.trim());
            setAcceptMsg(`paired with ${v.node_id_hex.slice(0, 12)}…`);
            setPaste("");
            await refresh();
        } catch (e) {
            setAcceptErr(String(e));
        } finally {
            setBusy(false);
        }
    };

    const forget = async (nodeIdHex: string) => {
        if (!confirm(`Forget peer ${nodeIdHex.slice(0, 12)}…?`)) return;
        try {
            await api.peerForget(nodeIdHex);
            await refresh();
        } catch (e) {
            console.warn("peerForget failed", e);
        }
    };

    return (
        <div className="peers">
            <h2>Peers</h2>
            <p className="peers-intro">
                Exchange a pairing ticket to let another Media Vault device see
                this one. Ticket contains your node + identity public keys and a
                signed timestamp — no network call until you share an album.
            </p>

            <section className="peers-my-ticket">
                <h3>My pairing ticket</h3>
                {!ticket && (
                    <button onClick={generateTicket} disabled={busy}>
                        {busy ? "Generating…" : "Generate ticket"}
                    </button>
                )}
                {ticket && (
                    <div className="ticket-card" data-testid="peers-ticket-card">
                        <textarea
                            readOnly
                            rows={4}
                            value={ticket.base32}
                            onFocus={(e) => e.currentTarget.select()}
                            data-testid="peers-ticket-base32"
                        />
                        <div className="ticket-meta">
                            <div>
                                <span className="label">my node</span>
                                <code>{ticket.my_node_id_hex.slice(0, 16)}…</code>
                            </div>
                            <div>
                                <span className="label">issued</span>
                                <code>{new Date(ticket.created_at * 1000).toISOString()}</code>
                            </div>
                            <button
                                onClick={() => {
                                    void navigator.clipboard.writeText(ticket.base32);
                                }}
                            >
                                Copy
                            </button>
                            <button onClick={generateTicket} disabled={busy}>
                                Regenerate
                            </button>
                        </div>
                    </div>
                )}
                {ticketErr && <p className="error">{ticketErr}</p>}
            </section>

            <section className="peers-accept">
                <h3>Accept a peer</h3>
                <label htmlFor="peers-paste">Paste their ticket</label>
                <textarea
                    id="peers-paste"
                    rows={4}
                    value={paste}
                    onChange={(e) => setPaste(e.target.value)}
                    disabled={busy}
                    placeholder="base32…"
                />
                <button onClick={acceptPaste} disabled={busy || !paste.trim()}>
                    Accept
                </button>
                {acceptErr && <p className="error">{acceptErr}</p>}
                {acceptMsg && <p className="success">{acceptMsg}</p>}
            </section>

            <section className="peers-list">
                <h3>Paired peers</h3>
                {accepted.length === 0 && <p>(no peers yet)</p>}
                <ul>
                    {accepted.map((p) => (
                        <li key={p.node_id_hex}>
                            <div className="peer-row">
                                <code>{p.node_id_hex.slice(0, 16)}…</code>
                                <span className="relay">
                                    {p.relay_url ?? "LAN only"}
                                </span>
                                <span className="added">
                                    {new Date(p.added_at * 1000).toLocaleString()}
                                </span>
                                <button onClick={() => void forget(p.node_id_hex)}>
                                    Forget
                                </button>
                            </div>
                        </li>
                    ))}
                </ul>
            </section>
        </div>
    );
}
