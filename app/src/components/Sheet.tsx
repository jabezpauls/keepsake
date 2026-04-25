import * as RadixDialog from "@radix-ui/react-dialog";
import { ReactNode } from "react";
import { cn } from "./cn";

interface SheetProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    /** Edge to slide in from. Default: "right". */
    side?: "right" | "left" | "bottom";
    title?: ReactNode;
    description?: ReactNode;
    children: ReactNode;
    actions?: ReactNode;
    className?: string;
}

// Side-anchored panel for settings, share, or context-rich detail
// (vs. a centered Modal). Reuses Radix Dialog so focus management +
// scroll lock + Esc all work automatically.
export function Sheet({
    open,
    onOpenChange,
    side = "right",
    title,
    description,
    children,
    actions,
    className,
}: SheetProps) {
    return (
        <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
            <RadixDialog.Portal>
                <RadixDialog.Overlay className="kp-overlay" />
                <RadixDialog.Content
                    className={cn("kp-sheet", className)}
                    data-side={side}
                >
                    <div className="kp-sheet-body">
                        {title && (
                            <RadixDialog.Title className="kp-sheet-title">
                                {title}
                            </RadixDialog.Title>
                        )}
                        {description && (
                            <RadixDialog.Description className="kp-sheet-description">
                                {description}
                            </RadixDialog.Description>
                        )}
                        {children}
                    </div>
                    {actions && <div className="kp-sheet-actions">{actions}</div>}
                </RadixDialog.Content>
            </RadixDialog.Portal>
        </RadixDialog.Root>
    );
}
