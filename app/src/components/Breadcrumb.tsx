import { Fragment, ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "./cn";

export interface BreadcrumbItem {
    label: ReactNode;
    onClick?: () => void;
}

interface BreadcrumbProps {
    items: BreadcrumbItem[];
    className?: string;
}

// Stack breadcrumb. The last item is the current location (no click).
// Earlier items are buttons that pop the back-stack to that level.
// Wired up by Phase 2's navigation.ts (one click → setView for that
// stack frame).
export function Breadcrumb({ items, className }: BreadcrumbProps) {
    if (items.length === 0) return null;
    return (
        <ol className={cn("kp-breadcrumb", className)}>
            {items.map((item, i) => {
                const isLast = i === items.length - 1;
                return (
                    <Fragment key={i}>
                        <li className="kp-breadcrumb-item">
                            {isLast || !item.onClick ? (
                                <span className="kp-breadcrumb-current">
                                    {item.label}
                                </span>
                            ) : (
                                <button
                                    type="button"
                                    className="kp-breadcrumb-link"
                                    onClick={item.onClick}
                                >
                                    {item.label}
                                </button>
                            )}
                        </li>
                        {!isLast && (
                            <ChevronRight
                                size={12}
                                aria-hidden
                                className="kp-breadcrumb-separator"
                            />
                        )}
                    </Fragment>
                );
            })}
        </ol>
    );
}
