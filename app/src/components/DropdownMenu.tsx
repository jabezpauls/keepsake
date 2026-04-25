import * as RadixDropdownMenu from "@radix-ui/react-dropdown-menu";
import { ReactNode } from "react";
import { cn } from "./cn";

interface DropdownMenuProps {
    trigger: ReactNode;
    children: ReactNode;
    side?: "top" | "right" | "bottom" | "left";
    align?: "start" | "center" | "end";
}

// Dropdown menu wrapper. Use for overflow actions on cards / toolbars.
// Children are <DropdownItem> / <DropdownSeparator>; the menu handles
// keyboard navigation, focus management, and portaled rendering.
export function DropdownMenu({
    trigger,
    children,
    side = "bottom",
    align = "end",
}: DropdownMenuProps) {
    return (
        <RadixDropdownMenu.Root>
            <RadixDropdownMenu.Trigger asChild>{trigger}</RadixDropdownMenu.Trigger>
            <RadixDropdownMenu.Portal>
                <RadixDropdownMenu.Content
                    side={side}
                    align={align}
                    sideOffset={4}
                    className="kp-menu"
                >
                    {children}
                </RadixDropdownMenu.Content>
            </RadixDropdownMenu.Portal>
        </RadixDropdownMenu.Root>
    );
}

interface DropdownItemProps {
    children: ReactNode;
    onSelect?: () => void;
    disabled?: boolean;
    icon?: ReactNode;
    variant?: "default" | "danger";
    className?: string;
}

export function DropdownItem({
    children,
    onSelect,
    disabled,
    icon,
    variant = "default",
    className,
}: DropdownItemProps) {
    return (
        <RadixDropdownMenu.Item
            disabled={disabled}
            onSelect={onSelect}
            className={cn("kp-menu-item", className)}
            data-variant={variant}
        >
            {icon}
            {children}
        </RadixDropdownMenu.Item>
    );
}

export function DropdownSeparator() {
    return <RadixDropdownMenu.Separator className="kp-menu-separator" />;
}
