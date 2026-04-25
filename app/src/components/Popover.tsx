import * as RadixPopover from "@radix-ui/react-popover";
import { ReactNode } from "react";

interface PopoverProps {
    /** Trigger element — typically a button. Use asChild semantics. */
    trigger: ReactNode;
    children: ReactNode;
    side?: "top" | "right" | "bottom" | "left";
    align?: "start" | "center" | "end";
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
}

// Click-triggered floating panel for non-modal flyouts (EXIF preview,
// photo info, etc.). Differs from Tooltip in three ways: opens on
// click not hover, supports rich content, and traps focus while open.
export function Popover({
    trigger,
    children,
    side = "bottom",
    align = "center",
    open,
    onOpenChange,
}: PopoverProps) {
    return (
        <RadixPopover.Root open={open} onOpenChange={onOpenChange}>
            <RadixPopover.Trigger asChild>{trigger}</RadixPopover.Trigger>
            <RadixPopover.Portal>
                <RadixPopover.Content
                    side={side}
                    align={align}
                    sideOffset={6}
                    className="kp-popover"
                >
                    {children}
                </RadixPopover.Content>
            </RadixPopover.Portal>
        </RadixPopover.Root>
    );
}
