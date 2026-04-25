import { forwardRef, HTMLAttributes, ReactNode } from "react";
import PersonFaceThumb from "../ui/people/PersonFaceThumb";
import { cn } from "./cn";

export type AvatarSize = "xs" | "sm" | "md" | "lg" | "xl";

interface AvatarBaseProps extends HTMLAttributes<HTMLDivElement> {
    size?: AvatarSize;
    alt?: string;
}

interface AvatarPersonProps extends AvatarBaseProps {
    /** Backend person id — fetches the highest-quality face thumb via IPC. */
    personId: number;
}

interface AvatarFallbackProps extends AvatarBaseProps {
    /** 1-2 character fallback when no thumb is available (initials). */
    fallback: string;
}

interface AvatarCustomProps extends AvatarBaseProps {
    /** Render arbitrary content (e.g. a static <img> or icon). */
    children: ReactNode;
}

type AvatarProps = AvatarPersonProps | AvatarFallbackProps | AvatarCustomProps;

const SIZE_PX: Record<AvatarSize, number> = {
    xs: 16,
    sm: 24,
    md: 40,
    lg: 64,
    xl: 96,
};

function isPerson(props: AvatarProps): props is AvatarPersonProps {
    return "personId" in props;
}

function isFallback(props: AvatarProps): props is AvatarFallbackProps {
    return "fallback" in props;
}

// Circular avatar primitive. Three variants:
//   - personId  → loads the cropped face via PersonFaceThumb
//   - fallback  → renders 1-2 character initials
//   - children  → arbitrary content
//
// Destructuring strips the discriminator fields (personId / fallback /
// children) before spreading the rest onto the div, otherwise React
// warns about unknown DOM attributes like `personid`.
export const Avatar = forwardRef<HTMLDivElement, AvatarProps>(function Avatar(
    props,
    ref,
) {
    if (isPerson(props)) {
        const { size = "md", alt, className, personId, ...rest } = props;
        return (
            <div
                ref={ref}
                className={cn("kp-avatar", className)}
                data-size={size}
                {...rest}
            >
                <PersonFaceThumb
                    personId={personId}
                    size={SIZE_PX[size] * 2}
                    alt={alt}
                />
            </div>
        );
    }
    if (isFallback(props)) {
        const { size = "md", alt, className, fallback, ...rest } = props;
        return (
            <div
                ref={ref}
                className={cn("kp-avatar", className)}
                data-size={size}
                {...rest}
            >
                <div className="kp-avatar-fallback" aria-label={alt}>
                    {fallback.slice(0, 2)}
                </div>
            </div>
        );
    }
    const { size = "md", className, children, alt: _alt, ...rest } = props;
    return (
        <div
            ref={ref}
            className={cn("kp-avatar", className)}
            data-size={size}
            {...rest}
        >
            {children}
        </div>
    );
});
