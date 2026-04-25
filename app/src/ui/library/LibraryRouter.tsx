import Library from "./Library";

// Routes the "library" view to the Phase 3 Library component.
// (Earlier phases routed here too, with a Timeline placeholder; the
// router stays in place so future Library-zone modes — Map, etc. —
// can fork off it.)
export default function LibraryRouter() {
    return <Library />;
}
