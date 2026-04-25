import { useEffect, useState, KeyboardEvent } from "react";
import {
    BookHeart,
    Camera,
    ChevronsLeft,
    ChevronsRight,
    Copy,
    FolderOpen,
    Images,
    Lock,
    MapPin,
    PawPrint,
    Plane,
    Search,
    Settings,
    Sparkles,
    Star,
    Users,
} from "lucide-react";
import { useSession } from "../../state/session";
import { Tooltip, cn } from "../../components";
import { StatusOrb } from "./StatusOrb";

interface SidebarProps {
    onOpenCommand: () => void;
    onOpenSettings: () => void;
    onLock: () => void;
    onLogoLongPress: () => void;
}

// Persistent left sidebar, the new IA's primary navigation. Four zones at
// the top, a collapsible Pinned section in the middle (with quick links to
// the legacy single-purpose screens until Phase 6 absorbs them into Albums),
// and a footer with ⌘K hint, Settings gear, ML orb, and Lock.
//
// Width: 232 px expanded, 56 px collapsed (icon-only). Collapse state
// persists in localStorage as `mv-sidebar-collapsed` so the layout choice
// survives reloads.
export function Sidebar({
    onOpenCommand,
    onOpenSettings,
    onLock,
    onLogoLongPress,
}: SidebarProps) {
    const view = useSession((s) => s.view);
    const setView = useSession((s) => s.setView);
    const hiddenUnlocked = useSession((s) => s.hiddenUnlocked);

    const [collapsed, setCollapsed] = useState<boolean>(() => {
        try {
            return localStorage.getItem("mv-sidebar-collapsed") === "1";
        } catch {
            return false;
        }
    });

    useEffect(() => {
        try {
            localStorage.setItem("mv-sidebar-collapsed", collapsed ? "1" : "0");
        } catch {
            // Ignore (private mode, quota).
        }
    }, [collapsed]);

    // Long-press the logo to enter the hidden vault. Mirrors the existing
    // Unlock.tsx affordance so the discovery surface stays the same.
    const longPressHandlers = useLongPress(onLogoLongPress);

    const isLibrary = view.kind === "library" || view.kind === "timeline" || view.kind === "map";
    const isForYou = view.kind === "for-you" || view.kind === "memories";
    const isAlbums =
        view.kind === "albums" ||
        view.kind === "album" ||
        view.kind === "smart_albums" ||
        view.kind === "smart_album" ||
        view.kind === "people" ||
        view.kind === "person" ||
        view.kind === "pets" ||
        view.kind === "trips" ||
        view.kind === "duplicates";
    const isSearch = view.kind === "search";

    return (
        <nav className="kp-sidebar" data-collapsed={collapsed ? "true" : undefined}>
            <div
                className="kp-sidebar-logo"
                {...longPressHandlers}
                role="button"
                tabIndex={0}
                aria-label="Keepsake — long-press for hidden vault"
            >
                <BookHeart size={20} />
                {!collapsed && <span>Keepsake</span>}
                {hiddenUnlocked && <span className="kp-sidebar-hidden-dot" aria-label="Hidden vault unlocked" />}
            </div>

            <div className="kp-sidebar-divider" />

            <ul className="kp-sidebar-zones">
                <ZoneButton
                    icon={<Images size={18} />}
                    label="Library"
                    active={isLibrary}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "library" })}
                />
                <ZoneButton
                    icon={<Sparkles size={18} />}
                    label="For You"
                    active={isForYou}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "for-you" })}
                />
                <ZoneButton
                    icon={<FolderOpen size={18} />}
                    label="Albums"
                    active={isAlbums}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "albums" })}
                />
                <ZoneButton
                    icon={<Search size={18} />}
                    label="Search"
                    active={isSearch}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "search" })}
                />
            </ul>

            {!collapsed && <div className="kp-sidebar-section-label">PINNED</div>}
            <ul className="kp-sidebar-pinned">
                <PinnedLink
                    icon={<Users size={16} />}
                    label="People"
                    active={view.kind === "people" || view.kind === "person"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "people" })}
                />
                <PinnedLink
                    icon={<PawPrint size={16} />}
                    label="Pets"
                    active={view.kind === "pets"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "pets" })}
                />
                <PinnedLink
                    icon={<Plane size={16} />}
                    label="Trips"
                    active={view.kind === "trips"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "trips" })}
                />
                <PinnedLink
                    icon={<MapPin size={16} />}
                    label="Map"
                    active={view.kind === "map"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "map" })}
                />
                <PinnedLink
                    icon={<Star size={16} />}
                    label="Smart albums"
                    active={view.kind === "smart_albums" || view.kind === "smart_album"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "smart_albums" })}
                />
                <PinnedLink
                    icon={<Copy size={16} />}
                    label="Duplicates"
                    active={view.kind === "duplicates"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "duplicates" })}
                />
                <PinnedLink
                    icon={<Camera size={16} />}
                    label="Memories"
                    active={view.kind === "memories"}
                    collapsed={collapsed}
                    onClick={() => setView({ kind: "memories" })}
                />
            </ul>

            <div className="kp-sidebar-spacer" />

            <ul className="kp-sidebar-footer">
                {!collapsed && (
                    <li>
                        <button
                            type="button"
                            className="kp-sidebar-cmdk-hint"
                            onClick={onOpenCommand}
                        >
                            <span>Quick search</span>
                            <kbd>⌘K</kbd>
                        </button>
                    </li>
                )}
                <li className="kp-sidebar-tray">
                    <Tooltip content="Settings">
                        <button
                            type="button"
                            className="kp-icon-button"
                            data-size="md"
                            onClick={onOpenSettings}
                            aria-label="Settings"
                        >
                            <Settings size={16} />
                        </button>
                    </Tooltip>
                    <StatusOrb onClick={onOpenSettings} />
                    <Tooltip content="Lock vault">
                        <button
                            type="button"
                            className="kp-icon-button"
                            data-size="md"
                            onClick={onLock}
                            aria-label="Lock vault"
                        >
                            <Lock size={16} />
                        </button>
                    </Tooltip>
                    <span className="kp-sidebar-tray-spacer" />
                    <Tooltip content={collapsed ? "Expand sidebar" : "Collapse sidebar"}>
                        <button
                            type="button"
                            className="kp-icon-button"
                            data-size="sm"
                            onClick={() => setCollapsed((c) => !c)}
                            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
                        >
                            {collapsed ? <ChevronsRight size={14} /> : <ChevronsLeft size={14} />}
                        </button>
                    </Tooltip>
                </li>
            </ul>
        </nav>
    );
}

interface ZoneButtonProps {
    icon: React.ReactNode;
    label: string;
    active: boolean;
    collapsed: boolean;
    onClick: () => void;
}

function ZoneButton({ icon, label, active, collapsed, onClick }: ZoneButtonProps) {
    const button = (
        <button
            type="button"
            className={cn("kp-sidebar-zone")}
            data-active={active ? "true" : undefined}
            onClick={onClick}
        >
            {icon}
            {!collapsed && <span>{label}</span>}
        </button>
    );
    if (collapsed) {
        return <li><Tooltip content={label} side="right">{button}</Tooltip></li>;
    }
    return <li>{button}</li>;
}

function PinnedLink({ icon, label, active, collapsed, onClick }: ZoneButtonProps) {
    const button = (
        <button
            type="button"
            className="kp-sidebar-pinned-item"
            data-active={active ? "true" : undefined}
            onClick={onClick}
        >
            {icon}
            {!collapsed && <span>{label}</span>}
        </button>
    );
    if (collapsed) {
        return <li><Tooltip content={label} side="right">{button}</Tooltip></li>;
    }
    return <li>{button}</li>;
}

// 600 ms long-press detector, used for the hidden-vault entry on the logo.
// Returns spread-able mouse + touch + key handlers.
function useLongPress(onTrigger: () => void) {
    const [timer, setTimer] = useState<number | null>(null);
    const start = () => {
        const t = window.setTimeout(onTrigger, 600);
        setTimer(t);
    };
    const cancel = () => {
        if (timer != null) {
            window.clearTimeout(timer);
            setTimer(null);
        }
    };
    return {
        onMouseDown: start,
        onMouseUp: cancel,
        onMouseLeave: cancel,
        onTouchStart: start,
        onTouchEnd: cancel,
        onTouchCancel: cancel,
        onKeyDown: (e: KeyboardEvent) => {
            if (e.key === "Enter" && !timer) start();
        },
        onKeyUp: cancel,
    };
}

// `ZoneButtonProps` is reused in PinnedLink — kept private to this module.
export type { SidebarProps };
