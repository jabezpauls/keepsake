import * as RadixTooltip from "@radix-ui/react-tooltip";
import { ReactNode } from "react";

interface TooltipProps {
    content: ReactNode;
    children: ReactNode;
    /** Side of the trigger to render on. Default: "top". */
    side?: "top" | "right" | "bottom" | "left";
    /** Delay before showing the tooltip on hover (ms). Default: 300. */
    delay?: number;
}

// Lightweight tooltip wrapper around Radix. Provider lives at the app
// root (mounted in App.tsx); this component just gates the trigger
// + content. Tooltips are essential for IconButton — never ship an
// icon-only button without one.
export function Tooltip({ content, children, side = "top", delay = 300 }: TooltipProps) {
    return (
        <RadixTooltip.Root delayDuration={delay}>
            <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
            <RadixTooltip.Portal>
                <RadixTooltip.Content
                    side={side}
                    sideOffset={6}
                    className="kp-tooltip"
                >
                    {content}
                </RadixTooltip.Content>
            </RadixTooltip.Portal>
        </RadixTooltip.Root>
    );
}

// Re-export the provider so App.tsx (or the gallery) can mount it once
// at the root. All Tooltip uses share that provider.
export const TooltipProvider = RadixTooltip.Provider;
