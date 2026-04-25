import { forwardRef, MouseEvent, ReactNode } from "react";
import {
    Aperture,
    Calendar,
    Camera,
    Folder,
    MapPin,
    Plane,
    Share2,
    Tag,
    User,
} from "lucide-react";
import { Avatar } from "./Avatar";
import { cn } from "./cn";

// Discriminated union of every kind of entity Keepsake can link to.
// Phase 4's `navigateToEntity(entity, currentView)` dispatcher is the
// other half — it takes one of these and returns the View to push onto
// the back-stack. Keep these shapes additive: adding a new kind here
// doesn't break callers that pattern-match the existing kinds.
export type Entity =
    | { kind: "person"; id: number; name: string | null }
    | { kind: "place"; placeId: string; name: string }
    | {
          kind: "date";
          utcDay: number;
          label: string;
          /** "exact" | "month" | "year" — affects the rendering format. */
          precision?: "exact" | "month" | "year" | "today";
      }
    | { kind: "trip"; id: number; name: string }
    | { kind: "album"; id: number; name: string }
    | { kind: "peer"; nodeIdHex: string; label?: string }
    | { kind: "camera"; make: string }
    | { kind: "lens"; lens: string }
    | {
          kind: "category";
          key:
              | "video"
              | "raw"
              | "screenshot"
              | "live"
              | "selfie"
              | "document"
              | "panorama"
              | "burst"
              | "long_exposure";
          label: string;
      };

interface EntityChipProps {
    entity: Entity;
    size?: "sm" | "md";
    /** Click handler — typically wired to `navigateToEntity`. */
    onClick?: (entity: Entity, e: MouseEvent<HTMLButtonElement>) => void;
    className?: string;
}

interface RenderInfo {
    icon: ReactNode;
    label: string;
}

function renderEntity(entity: Entity): RenderInfo {
    const iconSize = 12;
    switch (entity.kind) {
        case "person":
            return {
                icon: <User size={iconSize} aria-hidden />,
                label: entity.name ?? "Unnamed person",
            };
        case "place":
            return {
                icon: <MapPin size={iconSize} aria-hidden />,
                label: entity.name,
            };
        case "date":
            return {
                icon: <Calendar size={iconSize} aria-hidden />,
                label: entity.label,
            };
        case "trip":
            return {
                icon: <Plane size={iconSize} aria-hidden />,
                label: entity.name,
            };
        case "album":
            return {
                icon: <Folder size={iconSize} aria-hidden />,
                label: entity.name,
            };
        case "peer":
            return {
                icon: <Share2 size={iconSize} aria-hidden />,
                label:
                    entity.label ??
                    `${entity.nodeIdHex.slice(0, 4)}…${entity.nodeIdHex.slice(-4)}`,
            };
        case "camera":
            return {
                icon: <Camera size={iconSize} aria-hidden />,
                label: entity.make,
            };
        case "lens":
            return {
                icon: <Aperture size={iconSize} aria-hidden />,
                label: entity.lens,
            };
        case "category":
            return {
                icon: <Tag size={iconSize} aria-hidden />,
                label: entity.label,
            };
    }
}

// Single primitive that renders any Entity as a clickable chip. Kind-
// specific affordances (e.g. an avatar for person) are added inline.
// The chip never knows where it routes to — callers wire `onClick` to
// the navigation dispatcher.
export const EntityChip = forwardRef<HTMLButtonElement, EntityChipProps>(
    function EntityChip({ entity, size = "sm", onClick, className }, ref) {
        const { icon, label } = renderEntity(entity);
        const isPersonAvatar = entity.kind === "person" && entity.id > 0;
        return (
            <button
                ref={ref}
                type="button"
                className={cn("kp-entity-chip", className)}
                data-kind={entity.kind}
                data-size={size}
                onClick={onClick ? (e) => onClick(entity, e) : undefined}
                disabled={!onClick}
            >
                {isPersonAvatar ? (
                    <Avatar
                        size="xs"
                        personId={(entity as { id: number }).id}
                        alt={label}
                    />
                ) : (
                    icon
                )}
                <span>{label}</span>
            </button>
        );
    },
);
