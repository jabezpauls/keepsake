// Zustand store: session + navigation + unlocked-album set + hidden state.
import { create } from "zustand";
import type { SessionHandle } from "../bindings/SessionHandle";

export type View =
    | { kind: "timeline" }
    | { kind: "sources" }
    | { kind: "albums" }
    | { kind: "album"; id: number; name: string }
    | { kind: "asset"; id: number; back: View };

interface SessionStore {
    session: SessionHandle | null;
    view: View;
    hiddenUnlocked: boolean;
    unlockedAlbums: Set<number>;

    setSession: (s: SessionHandle | null) => void;
    setView: (v: View) => void;
    setHiddenUnlocked: (b: boolean) => void;
    markAlbumUnlocked: (id: number) => void;
    reset: () => void;
}

export const useSession = create<SessionStore>((set) => ({
    session: null,
    view: { kind: "timeline" },
    hiddenUnlocked: false,
    unlockedAlbums: new Set(),

    setSession: (s) => set({ session: s }),
    setView: (v) => set({ view: v }),
    setHiddenUnlocked: (b) => set({ hiddenUnlocked: b }),
    markAlbumUnlocked: (id) =>
        set((st) => ({ unlockedAlbums: new Set(st.unlockedAlbums).add(id) })),
    reset: () =>
        set({
            session: null,
            view: { kind: "timeline" },
            hiddenUnlocked: false,
            unlockedAlbums: new Set(),
        }),
}));
