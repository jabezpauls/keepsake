import { ButtonHTMLAttributes, forwardRef, HTMLAttributes, ReactNode } from "react";
import { cn } from "./cn";

export type ChipSize = "sm" | "md" | "lg";

interface ChipBaseProps {
    size?: ChipSize;
    active?: boolean;
    leadingIcon?: ReactNode;
    children: ReactNode;
}

type ChipStaticProps = ChipBaseProps & HTMLAttributes<HTMLSpanElement>;

interface ChipButtonProps
    extends ChipBaseProps,
        Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
    /** Truthy = renders as a <button>; falsy = renders as a <span>. */
    onClick: ButtonHTMLAttributes<HTMLButtonElement>["onClick"];
}

type ChipProps = ChipStaticProps | ChipButtonProps;

function isClickable(props: ChipProps): props is ChipButtonProps {
    return typeof (props as ChipButtonProps).onClick === "function";
}

// Pill-shaped tag. Use `onClick` to make it interactive; otherwise it
// renders as a span. Style variants drive visual weight (active = filled
// accent, neutral = outlined).
export const Chip = forwardRef<HTMLElement, ChipProps>(function Chip(props, ref) {
    if (isClickable(props)) {
        const { size = "md", active, leadingIcon, children, className, ...rest } = props;
        return (
            <button
                ref={ref as React.Ref<HTMLButtonElement>}
                type="button"
                className={cn("kp-chip", className)}
                data-size={size}
                data-active={active ? "true" : undefined}
                data-clickable="true"
                {...rest}
            >
                {leadingIcon}
                {children}
            </button>
        );
    }
    const { size = "md", active, leadingIcon, children, className, ...rest } = props;
    return (
        <span
            ref={ref as React.Ref<HTMLSpanElement>}
            className={cn("kp-chip", className)}
            data-size={size}
            data-active={active ? "true" : undefined}
            {...rest}
        >
            {leadingIcon}
            {children}
        </span>
    );
});
