import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../ipc";
import type { AlbumView } from "../../bindings/AlbumView";
import { useSession } from "../../state/session";

export default function Albums() {
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);
    const markAlbumUnlocked = useSession((s) => s.markAlbumUnlocked);
    const queryClient = useQueryClient();

    const albums = useQuery<AlbumView[]>({
        queryKey: ["albums", hiddenUnlocked ? "withHidden" : "plain"],
        queryFn: () => api.listAlbums(hiddenUnlocked),
    });

    const [newName, setNewName] = useState("");
    const [newPassword, setNewPassword] = useState("");
    const [busy, setBusy] = useState(false);

    const create = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!newName.trim()) return;
        setBusy(true);
        try {
            await api.createAlbum(newName, newPassword.trim() ? newPassword : null);
            setNewName("");
            setNewPassword("");
            await queryClient.invalidateQueries({ queryKey: ["albums"] });
        } finally {
            setBusy(false);
        }
    };

    const unlockAlbum = async (album: AlbumView) => {
        const pw = window.prompt(`Password for "${album.name}"`);
        if (!pw) return;
        const ok = await api.unlockAlbum(album.id, pw);
        if (ok) {
            markAlbumUnlocked(album.id);
            await queryClient.invalidateQueries({ queryKey: ["albums"] });
        }
        // Wrong password: do nothing visible (§9 plausible-deniability).
    };

    return (
        <div className="albums">
            <h2>Albums</h2>
            <form onSubmit={create} className="new-album">
                <input
                    placeholder="New album name"
                    value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    disabled={busy}
                />
                <input
                    type="password"
                    placeholder="password (optional)"
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    disabled={busy}
                />
                <button type="submit" disabled={busy}>
                    Create
                </button>
            </form>

            {albums.isLoading && <p>Loading…</p>}
            <ul className="album-list">
                {albums.data?.map((a) => (
                    <li key={a.id} className="album-row">
                        <button
                            className="album-open"
                            onClick={() => setView({ kind: "album", id: a.id, name: a.name })}
                        >
                            <strong>{a.name}</strong>
                            <span className="count">{a.member_count} items</span>
                        </button>
                        {a.has_password && !a.unlocked && (
                            <button onClick={() => unlockAlbum(a)}>Unlock</button>
                        )}
                        {a.has_password && a.unlocked && <span className="pill">unlocked</span>}
                    </li>
                ))}
            </ul>
        </div>
    );
}
