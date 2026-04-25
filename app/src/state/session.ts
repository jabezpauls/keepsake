// Zustand store: session + navigation back-stack + unlocked-album set + hidden state.
//
// The View union has grown across phases:
//   Phase 2 added "for-you", "place", "settings", "library" stubs — they
//   coexist with the legacy "timeline", "memories", "people", … kinds so
//   feature screens can migrate one phase at a time.
//
// Navigation model:
//   `backstack` is the source of truth (first element = current view).
//   `view` is a derived shortcut for the top of the stack — kept as a
//   plain field rather than a selector to minimise the diff for screens
//   that read `useSession((s) => s.view)`.
//
// Three mutators:
//   - setView(v)   replaces the entire stack with [v]. Use for sidebar /
//                  zone navigation.
//   - pushView(v)  appends v on top. Use for drill-in (face chip → person,
//                  album row → album detail, etc.).
//   - popView()    removes the top frame and snaps to the new top. Use
//                  for breadcrumb back / Esc key.
import { create } from "zustand";
import type { SessionHandle } from "../bindings/SessionHandle";

export type AlbumSource = "user" | "smart" | "trip" | "person" | "pet";

export type View =
    // Zone defaults
    | { kind: "library" }
    | { kind: "for-you" }
    | { kind: "search" }

    // Legacy single-screen views — preserved across migrations
    | { kind: "timeline" }
    | { kind: "sources" }
    | { kind: "albums" }
    | { kind: "album"; id: number; name: string; source?: AlbumSource }
    | {
          kind: "asset";
          id: number;
          // `back` carries the parent view explicitly. Phase 2 retains
          // it so legacy AssetDetail keeps working without a refactor;
          // Phase 3's new AssetDetail will use the backstack instead.
          back: View;
          // Ordered list of sibling asset ids in the caller's grid, so
          // AssetDetail can offer arrow-key / chevron prev-next navigation.
          // Optional: callers that open the viewer without a grid context
          // (notifications, direct links) leave them unset and the viewer
          // degrades to a single-asset view.
          neighbors?: number[];
          index?: number;
      }
    | { kind: "map" }
    | { kind: "people" }
    | { kind: "person"; id: number; name: string | null }
    | { kind: "places" }
    | { kind: "place"; placeId: string; name: string }
    | { kind: "duplicates" }
    | { kind: "peers" }
    | { kind: "trips" }
    | { kind: "memories" }
    | { kind: "smart_albums" }
    | { kind: "smart_album"; id: number; name: string }
    | { kind: "pets" }
    | {
          kind: "settings";
          section?: "sources" | "peers" | "ml" | "appearance" | "vault";
      };

interface SessionStore {
    session: SessionHandle | null;
    /** The active view — always equals `backstack[backstack.length - 1]`. */
    view: View;
    /** Full navigation stack; first item is the deepest, last is current. */
    backstack: View[];
    hiddenUnlocked: boolean;
    unlockedAlbums: Set<number>;

    setSession: (s: SessionHandle | null) => void;
    /** Replace the entire stack with [v]. Used for sidebar zone navigation. */
    setView: (v: View) => void;
    /** Append v to the stack — for drill-in (chip click, asset detail). */
    pushView: (v: View) => void;
    /** Remove the top frame; if the stack would empty, no-op. */
    popView: () => void;
    setHiddenUnlocked: (b: boolean) => void;
    markAlbumUnlocked: (id: number) => void;
    reset: () => void;
}

// Phase 5 makes For-You the default landing — auto-curated memories
// + recent trips + featured people. Users with brand-new vaults get
// the empty-state CTA; everyone else gets the discovery surface.
const initialView: View = { kind: "for-you" };

export const useSession = create<SessionStore>((set) => ({
    session: null,
    view: initialView,
    backstack: [initialView],
    hiddenUnlocked: false,
    unlockedAlbums: new Set(),

    setSession: (s) => set({ session: s }),
    setView: (v) => set({ view: v, backstack: [v] }),
    pushView: (v) =>
        set((st) => ({ view: v, backstack: [...st.backstack, v] })),
    popView: () =>
        set((st) => {
            if (st.backstack.length <= 1) return st;
            const next = st.backstack.slice(0, -1);
            return { view: next[next.length - 1], backstack: next };
        }),
    setHiddenUnlocked: (b) => set({ hiddenUnlocked: b }),
    markAlbumUnlocked: (id) =>
        set((st) => ({ unlockedAlbums: new Set(st.unlockedAlbums).add(id) })),
    reset: () =>
        set({
            session: null,
            view: initialView,
            backstack: [initialView],
            hiddenUnlocked: false,
            unlockedAlbums: new Set(),
        }),
}));
