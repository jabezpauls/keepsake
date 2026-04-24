// Zustand store: session + navigation + unlocked-album set + hidden state.
import { create } from "zustand";
import type { SessionHandle } from "../bindings/SessionHandle";

export type View =
    | { kind: "timeline" }
    | { kind: "sources" }
    | { kind: "albums" }
    | { kind: "album"; id: number; name: string }
    | {
          kind: "asset";
          id: number;
          back: View;
          // Ordered list of sibling asset ids in the caller's grid, so
          // AssetDetail can offer arrow-key / chevron prev-next navigation.
          // Optional: callers that open the viewer without a grid context
          // (notifications, direct links) leave them unset and the viewer
          // degrades to a single-asset view.
          neighbors?: number[];
          index?: number;
      }
    | { kind: "search" }
    | { kind: "map" }
    | { kind: "people" }
    | { kind: "person"; id: number; name: string | null }
    | { kind: "duplicates" }
    | { kind: "peers" }
    | { kind: "trips" }
    | { kind: "memories" }
    | { kind: "smart_albums" }
    | { kind: "smart_album"; id: number; name: string };

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
