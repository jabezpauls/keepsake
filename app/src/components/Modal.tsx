import * as RadixDialog from "@radix-ui/react-dialog";
import { ReactNode } from "react";
import { cn } from "./cn";

interface ModalProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    title?: ReactNode;
    description?: ReactNode;
    children: ReactNode;
    actions?: ReactNode;
    /** Skip the centered content wrapper — caller provides full layout. */
    bare?: boolean;
    className?: string;
}

// Centered modal dialog. Uses Radix Dialog underneath for focus trap +
// scroll lock + Esc to close + portal rendering. The visible chrome
// (border, shadow, padding) is in primitives.css.
export function Modal({
    open,
    onOpenChange,
    title,
    description,
    children,
    actions,
    bare,
    className,
}: ModalProps) {
    return (
        <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
            <RadixDialog.Portal>
                <RadixDialog.Overlay className="kp-overlay" />
                <RadixDialog.Content className={cn("kp-modal", className)}>
                    {bare ? (
                        children
                    ) : (
                        <>
                            {title && (
                                <RadixDialog.Title className="kp-modal-title">
                                    {title}
                                </RadixDialog.Title>
                            )}
                            {description && (
                                <RadixDialog.Description className="kp-modal-description">
                                    {description}
                                </RadixDialog.Description>
                            )}
                            {children}
                            {actions && (
                                <div className="kp-modal-actions">{actions}</div>
                            )}
                        </>
                    )}
                </RadixDialog.Content>
            </RadixDialog.Portal>
        </RadixDialog.Root>
    );
}
