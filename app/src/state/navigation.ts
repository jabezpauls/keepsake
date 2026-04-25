// Single dispatcher: any Entity → the View it should drill into. Phase 4
// wires <EntityChip onClick={(entity) => pushView(entityToView(entity))}>
// across every surface; this module is where that mapping lives.
//
// Keeping this resolver in one file means future phases that add new
// view kinds (place screens, public-link viewer, etc.) only edit here,
// not every chip callsite.

import type { Entity } from "../components";
import type { View } from "./session";

/**
 * Resolve an entity reference into the view it should navigate to.
 * Returns `null` for entities that don't have a destination yet (e.g.
 * camera/lens/category, which are search-scope filters not screens).
 */
export function entityToView(entity: Entity): View | null {
    switch (entity.kind) {
        case "person":
            return { kind: "person", id: entity.id, name: entity.name };
        case "place":
            return { kind: "place", placeId: entity.placeId, name: entity.name };
        case "trip":
            // Trips are stored as collection(kind="trip"), so they open as albums.
            return {
                kind: "album",
                id: entity.id,
                name: entity.name,
                source: "trip",
            };
        case "album":
            return { kind: "album", id: entity.id, name: entity.name };
        case "peer":
            return { kind: "settings", section: "peers" };
        case "date":
            // Date chips re-scope search to that period. The query encoding is
            // handled by the search hub; for now we just route there.
            return { kind: "search" };
        case "camera":
        case "lens":
        case "category":
            // These are filter values, not destinations — the Search hub
            // will accept them as a scope param in Phase 7. For now, route
            // to Search and let the user manually re-filter.
            return { kind: "search" };
    }
}

/**
 * Convenience helper for chip onClick: pushes the entity's view onto the
 * navigation stack. Falls back to no-op if the entity has no destination.
 */
export function navigateToEntity(
    entity: Entity,
    pushView: (v: View) => void,
): void {
    const target = entityToView(entity);
    if (target) pushView(target);
}
