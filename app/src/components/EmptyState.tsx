import { ReactNode } from "react";
import { cn } from "./cn";

interface EmptyStateProps {
    icon?: ReactNode;
    title: ReactNode;
    hint?: ReactNode;
    actions?: ReactNode;
    className?: string;
}

// Centered empty-state block. Used everywhere a list/grid has no data.
// Pass a lucide icon (size={32}) for the icon slot. Actions render
// as buttons below the hint text.
export function EmptyState({ icon, title, hint, actions, className }: EmptyStateProps) {
    return (
        <div className={cn("kp-empty", className)}>
            {icon && <div className="kp-empty-icon">{icon}</div>}
            <h3 className="kp-empty-title">{title}</h3>
            {hint && <p className="kp-empty-hint">{hint}</p>}
            {actions && <div className="kp-empty-actions">{actions}</div>}
        </div>
    );
}
