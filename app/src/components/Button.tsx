import { ButtonHTMLAttributes, forwardRef, ReactNode } from "react";
import { cn } from "./cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
    variant?: ButtonVariant;
    size?: ButtonSize;
    leadingIcon?: ReactNode;
    trailingIcon?: ReactNode;
    loading?: boolean;
}

// Default button. `secondary` variant matches the legacy styling so screens
// can adopt incrementally without visual jank — a screen migrated to <Button>
// renders close enough to the old `button.muted-button` for the diff to read
// as polish, not redesign.
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
    {
        variant = "secondary",
        size = "md",
        leadingIcon,
        trailingIcon,
        loading,
        disabled,
        className,
        children,
        ...rest
    },
    ref,
) {
    return (
        <button
            ref={ref}
            className={cn("kp-button", className)}
            data-variant={variant}
            data-size={size}
            disabled={disabled || loading}
            {...rest}
        >
            {loading ? <span className="kp-spinner" aria-hidden /> : leadingIcon}
            {children}
            {trailingIcon}
        </button>
    );
});
