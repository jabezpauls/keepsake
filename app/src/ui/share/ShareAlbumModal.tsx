import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../ipc";
import type { PeerAcceptedView } from "../../bindings/PeerAcceptedView";
import type { ShareRecipientView } from "../../bindings/ShareRecipientView";

interface Props {
    albumId: number;
    albumName: string;
    onClose: () => void;
}

/**
 * Apple Photos-style share modal: lists paired peers alongside the
 * set currently holding a wrapping. Each row has a Share-or-Revoke
 * toggle. After sharing, the backend returns a namespace ticket the
 * user can send out-of-band (paste / QR); we surface it inline so the
 * user can copy it without leaving the modal.
 */
export default function ShareAlbumModal({ albumId, albumName, onClose }: Props) {
    const queryClient = useQueryClient();
    const peers = useQuery<PeerAcceptedView[]>({
        queryKey: ["peers"],
        queryFn: api.peerList,
    });
    const shares = useQuery<ShareRecipientView[]>({
        queryKey: ["album-shares", albumId],
        queryFn: () => api.listAlbumShares(albumId),
    });
    const [lastTicket, setLastTicket] = useState<string | null>(null);

    const shareMut = useMutation({
        mutationFn: async (nodeIdHex: string) =>
            api.shareAlbumWithPeer(albumId, nodeIdHex),
        onSuccess: (invite) => {
            setLastTicket(invite.namespace_ticket_base32);
            queryClient.invalidateQueries({
                queryKey: ["album-shares", albumId],
            });
        },
    });
    const revokeMut = useMutation({
        mutationFn: async (nodeIdHex: string) =>
            api.revokeAlbumShare(albumId, nodeIdHex),
        onSuccess: () => {
            queryClient.invalidateQueries({
                queryKey: ["album-shares", albumId],
            });
        },
    });

    const sharedSet = new Set(
        (shares.data ?? []).map((s) => s.peer_node_id_hex),
    );

    return (
        <div className="share-modal-backdrop" onClick={onClose}>
            <div
                className="share-modal"
                onClick={(e) => e.stopPropagation()}
                role="dialog"
                aria-label={`Share album "${albumName}"`}
            >
                <header>
                    <h2>Share “{albumName}”</h2>
                    <button className="close" onClick={onClose} aria-label="Close">
                        ×
                    </button>
                </header>

                <section>
                    <h3>Paired peers</h3>
                    {peers.isLoading && <p>Loading peers…</p>}
                    {peers.data && peers.data.length === 0 && (
                        <p>
                            No paired peers yet. Pair one on the Peers tab first.
                        </p>
                    )}
                    <ul className="share-peer-list">
                        {(peers.data ?? []).map((p) => {
                            const isShared = sharedSet.has(p.node_id_hex);
                            const busy =
                                shareMut.isPending || revokeMut.isPending;
                            return (
                                <li key={p.node_id_hex}>
                                    <div className="share-peer-row">
                                        <code>
                                            {p.node_id_hex.slice(0, 16)}…
                                        </code>
                                        <span className="relay">
                                            {p.relay_url ?? "LAN"}
                                        </span>
                                        {isShared ? (
                                            <button
                                                className="revoke"
                                                onClick={() =>
                                                    revokeMut.mutate(
                                                        p.node_id_hex,
                                                    )
                                                }
                                                disabled={busy}
                                            >
                                                {revokeMut.isPending
                                                    ? "Revoking…"
                                                    : "Revoke"}
                                            </button>
                                        ) : (
                                            <button
                                                className="share"
                                                onClick={() =>
                                                    shareMut.mutate(
                                                        p.node_id_hex,
                                                    )
                                                }
                                                disabled={busy}
                                            >
                                                {shareMut.isPending
                                                    ? "Sharing…"
                                                    : "Share"}
                                            </button>
                                        )}
                                    </div>
                                </li>
                            );
                        })}
                    </ul>
                </section>

                {lastTicket && (
                    <section className="share-ticket">
                        <h3>Namespace ticket</h3>
                        <p>
                            Send this ticket to the recipient so their device
                            can join the shared album.
                        </p>
                        <textarea
                            readOnly
                            rows={4}
                            value={lastTicket}
                            onFocus={(e) => e.currentTarget.select()}
                        />
                        <button
                            onClick={() => {
                                void navigator.clipboard.writeText(lastTicket);
                            }}
                        >
                            Copy
                        </button>
                    </section>
                )}

                <PublicLinksPanel albumId={albumId} />

                {(shareMut.isError || revokeMut.isError) && (
                    <p className="error">
                        {String(shareMut.error ?? revokeMut.error)}
                    </p>
                )}
            </div>
        </div>
    );
}

function PublicLinksPanel({ albumId }: { albumId: number }) {
    const queryClient = useQueryClient();
    const links = useQuery({
        queryKey: ["public-links"],
        queryFn: api.listPublicLinks,
    });
    const forThisAlbum = (links.data ?? []).filter(
        (l) => l.collection_id === albumId,
    );
    const [password, setPassword] = useState("");
    const [expiry, setExpiry] = useState<"never" | "7d" | "30d">("never");
    const [lastCreated, setLastCreated] = useState<
        import("../../bindings/PublicLinkView").PublicLinkView | null
    >(null);
    const [busy, setBusy] = useState(false);
    const [err, setErr] = useState<string | null>(null);

    const create = async () => {
        setBusy(true);
        setErr(null);
        try {
            const now = Math.floor(Date.now() / 1000);
            const expiresAt =
                expiry === "never"
                    ? null
                    : expiry === "7d"
                      ? now + 7 * 86400
                      : now + 30 * 86400;
            const link = await api.createPublicLink(
                albumId,
                password.trim() || null,
                expiresAt,
            );
            setLastCreated(link);
            setPassword("");
            await queryClient.invalidateQueries({ queryKey: ["public-links"] });
        } catch (e) {
            setErr(String(e));
        } finally {
            setBusy(false);
        }
    };

    const revoke = async (id: number) => {
        if (!window.confirm("Revoke this link? Anyone already holding it will lose access immediately.")) {
            return;
        }
        await api.revokePublicLink(id);
        if (lastCreated?.id === id) setLastCreated(null);
        await queryClient.invalidateQueries({ queryKey: ["public-links"] });
    };

    const hostBase = "https://<your-relay>/s/";
    const urlFor = (link: import("../../bindings/PublicLinkView").PublicLinkView) =>
        link.url_fragment
            ? `${hostBase}${link.pub_id_b32}#${link.url_fragment}`
            : `${hostBase}${link.pub_id_b32}`;

    return (
        <section className="public-links">
            <h3>Public link</h3>
            <p className="muted">
                Anyone with the URL (and password, if set) can view this album
                through the browser viewer. The viewer itself is a follow-up
                slice — the hosting peer's HTTP gateway isn't wired yet, so
                treat these as placeholders for now.
            </p>
            <div className="public-links-create">
                <input
                    type="password"
                    placeholder="Password (optional)"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    disabled={busy}
                />
                <select
                    value={expiry}
                    onChange={(e) => setExpiry(e.target.value as "never" | "7d" | "30d")}
                    disabled={busy}
                >
                    <option value="never">Never expires</option>
                    <option value="7d">Expires in 7 days</option>
                    <option value="30d">Expires in 30 days</option>
                </select>
                <button onClick={create} disabled={busy}>
                    {busy ? "Creating…" : "Create link"}
                </button>
            </div>
            {err && <p className="error">{err}</p>}
            {lastCreated && (
                <div className="public-link-new">
                    <strong>New link (copy now — fragment is not recoverable):</strong>
                    <textarea readOnly rows={3} value={urlFor(lastCreated)} onFocus={(e) => e.currentTarget.select()} />
                    <button
                        onClick={() => {
                            void navigator.clipboard.writeText(urlFor(lastCreated));
                        }}
                    >
                        Copy
                    </button>
                    {lastCreated.has_password && (
                        <p className="muted">
                            Recipient will be prompted for the password before
                            decryption.
                        </p>
                    )}
                </div>
            )}
            {forThisAlbum.length > 0 && (
                <ul className="public-link-list">
                    {forThisAlbum.map((l) => (
                        <li key={l.id}>
                            <code>{l.pub_id_b32.slice(0, 12)}…</code>
                            <span className="muted">
                                {l.has_password ? " · password" : " · fragment"}
                                {l.expires_at !== null && (
                                    <>
                                        {" · expires "}
                                        {new Date(l.expires_at * 1000).toLocaleDateString()}
                                    </>
                                )}
                            </span>
                            <button onClick={() => revoke(l.id)} className="revoke">
                                Revoke
                            </button>
                        </li>
                    ))}
                </ul>
            )}
        </section>
    );
}
