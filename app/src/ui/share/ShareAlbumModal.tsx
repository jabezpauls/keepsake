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

                {(shareMut.isError || revokeMut.isError) && (
                    <p className="error">
                        {String(shareMut.error ?? revokeMut.error)}
                    </p>
                )}
            </div>
        </div>
    );
}
