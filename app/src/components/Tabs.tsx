import * as RadixTabs from "@radix-ui/react-tabs";
import { ReactNode } from "react";
import { cn } from "./cn";

interface TabsProps {
    value: string;
    onValueChange: (value: string) => void;
    children: ReactNode;
    orientation?: "horizontal" | "vertical";
    className?: string;
}

// Controlled tabs wrapper. Caller owns the `value` and rerenders on
// change — matches our existing Zustand-based view-state pattern. Use
// vertical orientation for the Settings sheet, horizontal for For-You
// sub-sections etc.
export function Tabs({
    value,
    onValueChange,
    children,
    orientation = "horizontal",
    className,
}: TabsProps) {
    return (
        <RadixTabs.Root
            value={value}
            onValueChange={onValueChange}
            orientation={orientation}
            className={cn("kp-tabs", className)}
            data-orientation={orientation}
        >
            {children}
        </RadixTabs.Root>
    );
}

interface TabsListProps {
    children: ReactNode;
    className?: string;
}

export function TabsList({ children, className }: TabsListProps) {
    return (
        <RadixTabs.List className={cn("kp-tabs-list", className)}>
            {children}
        </RadixTabs.List>
    );
}

interface TabsTriggerProps {
    value: string;
    children: ReactNode;
    icon?: ReactNode;
    className?: string;
    disabled?: boolean;
}

export function TabsTrigger({
    value,
    children,
    icon,
    className,
    disabled,
}: TabsTriggerProps) {
    return (
        <RadixTabs.Trigger
            value={value}
            disabled={disabled}
            className={cn("kp-tabs-trigger", className)}
        >
            {icon}
            {children}
        </RadixTabs.Trigger>
    );
}

interface TabsContentProps {
    value: string;
    children: ReactNode;
    className?: string;
}

export function TabsContent({ value, children, className }: TabsContentProps) {
    return (
        <RadixTabs.Content
            value={value}
            className={cn("kp-tabs-content", className)}
        >
            {children}
        </RadixTabs.Content>
    );
}
