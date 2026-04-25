import { ButtonHTMLAttributes, forwardRef, ReactNode } from "react";
import { Tooltip } from "./Tooltip";
import { cn } from "./cn";

export type IconButtonSize = "sm" | "md" | "lg";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
    icon: ReactNode;
    /** Required: tooltip + accessible label. Screen readers announce this. */
    label: string;
    size?: IconButtonSize;
    active?: boolean;
    /** Skip the tooltip wrapper (use when consumer already provides one). */
    suppressTooltip?: boolean;
}

// Square button with an icon and a *required* label. Tooltip wraps it
// automatically — IconButtons without a visible label MUST always have a
// tooltip so screen readers and hover-discovery work for keyboard users.
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
    function IconButton(
        { icon, label, size = "md", active, suppressTooltip, className, ...rest },
        ref,
    ) {
        const button = (
            <button
                ref={ref}
                type="button"
                aria-label={label}
                className={cn("kp-icon-button", className)}
                data-size={size}
                data-active={active ? "true" : undefined}
                {...rest}
            >
                {icon}
            </button>
        );
        if (suppressTooltip) return button;
        return <Tooltip content={label}>{button}</Tooltip>;
    },
);
