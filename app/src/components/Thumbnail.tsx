import { forwardRef, HTMLAttributes } from "react";
import ThumbImage from "../ui/timeline/ThumbImage";
import { cn } from "./cn";

export type ThumbnailAspect = "square" | "16-9" | "4-3";

interface ThumbnailProps extends HTMLAttributes<HTMLDivElement> {
    assetId: number;
    /** Pixel size hint for the backend. Uses 2× for retina. */
    size: number;
    mime: string;
    aspect?: ThumbnailAspect;
    alt?: string;
}

// Rectangular thumbnail primitive. Wraps the legacy ThumbImage which
// already handles blob-URL revocation, fallback, and loading state.
// Use `aspect="square"` for grid cells, `aspect="16-9"` for hero cards.
export const Thumbnail = forwardRef<HTMLDivElement, ThumbnailProps>(
    function Thumbnail({ assetId, size, mime, aspect = "square", alt, className, ...rest }, ref) {
        return (
            <div
                ref={ref}
                className={cn("kp-thumbnail", className)}
                data-aspect={aspect}
                {...rest}
            >
                <ThumbImage assetId={assetId} size={size} mime={mime} alt={alt} />
            </div>
        );
    },
);
