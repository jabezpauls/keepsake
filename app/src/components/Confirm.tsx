import { ReactNode } from "react";
import { Modal } from "./Modal";
import { Button } from "./Button";

interface ConfirmProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    title: ReactNode;
    description?: ReactNode;
    confirmLabel?: string;
    cancelLabel?: string;
    /** Confirm button variant — "danger" for destructive operations. */
    variant?: "primary" | "danger";
    onConfirm: () => void;
}

// Modal preset for two-button confirmation. Replaces window.confirm()
// throughout the app — the legacy confirm dialogs broke focus, looked
// foreign, and couldn't be styled. Use `variant="danger"` for delete /
// revoke / forget flows.
export function Confirm({
    open,
    onOpenChange,
    title,
    description,
    confirmLabel = "Confirm",
    cancelLabel = "Cancel",
    variant = "primary",
    onConfirm,
}: ConfirmProps) {
    return (
        <Modal
            open={open}
            onOpenChange={onOpenChange}
            title={title}
            description={description}
            actions={
                <>
                    <Button variant="ghost" onClick={() => onOpenChange(false)}>
                        {cancelLabel}
                    </Button>
                    <Button
                        variant={variant === "danger" ? "danger" : "primary"}
                        onClick={() => {
                            onConfirm();
                            onOpenChange(false);
                        }}
                    >
                        {confirmLabel}
                    </Button>
                </>
            }
        >
            <div />
        </Modal>
    );
}
