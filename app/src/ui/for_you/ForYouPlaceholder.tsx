import { Sparkles } from "lucide-react";
import { Button, EmptyState } from "../../components";
import { useSession } from "../../state/session";

// Phase 2 placeholder for the For-You zone. Phase 5 replaces this with
// the full discovery surface (carousels for on-this-day, year-in-photos,
// person-year, recent trips, recent imports, featured people/places).
export default function ForYouPlaceholder() {
    const setView = useSession((s) => s.setView);
    return (
        <div style={{ padding: "var(--space-8) var(--space-6)" }}>
            <EmptyState
                icon={<Sparkles size={36} />}
                title="For You — coming in Phase 5"
                hint="Memories, year-in-photos, recent trips, and featured people will surface here. For now, the existing Memories screen has the same content."
                actions={
                    <>
                        <Button
                            variant="primary"
                            onClick={() => setView({ kind: "memories" })}
                        >
                            Open Memories
                        </Button>
                        <Button
                            variant="ghost"
                            onClick={() => setView({ kind: "library" })}
                        >
                            Go to Library
                        </Button>
                    </>
                }
            />
        </div>
    );
}
