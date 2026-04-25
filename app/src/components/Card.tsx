import { forwardRef, HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";

export type CardPadding = "none" | "sm" | "md" | "lg";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
    padding?: CardPadding;
    /** Apply hover-lift effect (translateY + shadow). Use for grid items. */
    hoverable?: boolean;
    /** Indicates the card is interactive (cursor + role=button). */
    onClick?: () => void;
    children: ReactNode;
}

// Surface container with consistent padding, border, and rounded corners.
// `hoverable` adds the signature lift used on Albums / For-You / Trips
// cards. Setting `onClick` automatically promotes the card to a button
// role for keyboard accessibility.
export const Card = forwardRef<HTMLDivElement, CardProps>(function Card(
    { padding = "md", hoverable, onClick, children, className, ...rest },
    ref,
) {
    return (
        <div
            ref={ref}
            className={cn("kp-card", className)}
            data-padding={padding}
            data-hoverable={hoverable ? "true" : undefined}
            data-clickable={onClick ? "true" : undefined}
            role={onClick ? "button" : undefined}
            tabIndex={onClick ? 0 : undefined}
            onClick={onClick}
            onKeyDown={
                onClick
                    ? (e) => {
                          if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              onClick();
                          }
                      }
                    : undefined
            }
            {...rest}
        >
            {children}
        </div>
    );
});
