import { useQuery } from "@tanstack/react-query";
import { api, bytesToBlobUrl } from "../../ipc";
import type { PetAssetView } from "../../bindings/PetAssetView";
import { useSession } from "../../state/session";

export default function Pets() {
    const setView = useSession((s) => s.setView);
    const pets = useQuery<PetAssetView[]>({
        queryKey: ["pets"],
        queryFn: api.listPetAssets,
    });

    // Group by species — untagged rows land under "Unspecified".
    const groups = new Map<string, PetAssetView[]>();
    for (const p of pets.data ?? []) {
        const key = p.species ?? "Unspecified";
        const existing = groups.get(key) ?? [];
        existing.push(p);
        groups.set(key, existing);
    }
    const speciesOrdered = Array.from(groups.keys()).sort();

    return (
        <div className="pets-view">
            <nav className="pets-nav">
                <h2>Pets</h2>
            </nav>
            <p className="muted">
                Assets you've flagged as pets. Auto-detection via an on-device
                classifier is a follow-up — for now, open any photo and click
                <em> Mark as pet</em> in its detail pane.
            </p>
            {pets.isLoading && <p>Loading…</p>}
            {pets.data && pets.data.length === 0 && (
                <p className="muted">No pets yet.</p>
            )}
            {speciesOrdered.map((species) => {
                const assets = groups.get(species)!;
                return (
                    <section key={species} className="pet-group">
                        <h3>
                            {species}{" "}
                            <span className="count">({assets.length})</span>
                        </h3>
                        <div className="pet-strip">
                            {assets.map((a, idx) => (
                                <PetThumb
                                    key={a.id}
                                    assetId={a.id}
                                    onOpen={() =>
                                        setView({
                                            kind: "asset",
                                            id: a.id,
                                            back: { kind: "pets" },
                                            neighbors: assets.map((x) => x.id),
                                            index: idx,
                                        })
                                    }
                                />
                            ))}
                        </div>
                    </section>
                );
            })}
        </div>
    );
}

function PetThumb({
    assetId,
    onOpen,
}: {
    assetId: number;
    onOpen: () => void;
}) {
    const thumb = useQuery({
        queryKey: ["pet-thumb", assetId],
        queryFn: () => api.assetThumbnail(assetId, 256),
        staleTime: 10 * 60_000,
    });
    const url = thumb.data ? bytesToBlobUrl(thumb.data, "image/webp") : null;
    return (
        <button
            className="pet-thumb"
            onClick={onOpen}
            aria-label={`Open pet photo ${assetId}`}
        >
            {url ? <img src={url} alt="" /> : <div className="pet-thumb-placeholder" />}
        </button>
    );
}
