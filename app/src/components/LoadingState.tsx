import { CSSProperties } from "react";
import { cn } from "./cn";

interface SkeletonProps {
    width?: string | number;
    height?: string | number;
    radius?: string | number;
    className?: string;
}

// Block-shaped placeholder with a shimmer animation. Use for thumbnails,
// list rows, text lines while data is loading. Looks better than a
// "Loading…" string and degrades cleanly under reduced-motion (the
// shimmer animation duration becomes 0).
export function Skeleton({ width, height, radius, className }: SkeletonProps) {
    const style: CSSProperties = {};
    if (width !== undefined) style.width = typeof width === "number" ? `${width}px` : width;
    if (height !== undefined) style.height = typeof height === "number" ? `${height}px` : height;
    if (radius !== undefined)
        style.borderRadius = typeof radius === "number" ? `${radius}px` : radius;
    return <div className={cn("kp-skeleton", className)} style={style} />;
}

// Tiny circular spinner. Use sparingly — Skeleton is preferred for
// content placeholders. Spinners are for "submitting" / "in flight"
// states inside buttons.
export function Spinner({ className }: { className?: string }) {
    return <span className={cn("kp-spinner", className)} aria-label="Loading" />;
}
