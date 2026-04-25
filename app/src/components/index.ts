// Public surface of the design system. Feature screens import from
// "@/components" rather than reaching into individual files; this keeps
// the migration log honest (one barrel = one diff).
export { Button } from "./Button";
export type { ButtonVariant, ButtonSize } from "./Button";

export { IconButton } from "./IconButton";
export type { IconButtonSize } from "./IconButton";

export { Chip } from "./Chip";
export type { ChipSize } from "./Chip";

export { Card } from "./Card";
export type { CardPadding } from "./Card";

export { Avatar } from "./Avatar";
export type { AvatarSize } from "./Avatar";

export { Thumbnail } from "./Thumbnail";
export type { ThumbnailAspect } from "./Thumbnail";

export { EntityChip } from "./EntityChip";
export type { Entity } from "./EntityChip";

export { Tooltip, TooltipProvider } from "./Tooltip";
export { Popover } from "./Popover";
export {
    DropdownMenu,
    DropdownItem,
    DropdownSeparator,
} from "./DropdownMenu";
export { Tabs, TabsList, TabsTrigger, TabsContent } from "./Tabs";
export { Modal } from "./Modal";
export { Sheet } from "./Sheet";
export { Confirm } from "./Confirm";
export { ToastProvider } from "./Toast";
export { useToast } from "./useToast";
export type { ToastVariant, ToastOptions } from "./useToast";
export { EmptyState } from "./EmptyState";
export { Skeleton, Spinner } from "./LoadingState";
export { Breadcrumb } from "./Breadcrumb";
export type { BreadcrumbItem } from "./Breadcrumb";

export { cn } from "./cn";
