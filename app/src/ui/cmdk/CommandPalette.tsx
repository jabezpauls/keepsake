import { Command } from "cmdk";
import { useEffect } from "react";
import {
    Camera,
    Copy,
    FolderOpen,
    Images,
    Lock,
    MapPin,
    PawPrint,
    Plane,
    RefreshCcw,
    Search,
    Settings,
    Sparkles,
    Star,
    Users,
} from "lucide-react";
import { useSession, View } from "../../state/session";
import { api } from "../../ipc";

interface CommandPaletteProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onOpenSettings: () => void;
    onOpenWizard: () => void;
}

interface NavCommand {
    id: string;
    label: string;
    icon: React.ReactNode;
    keywords: string;
    view?: View;
    /** Free-form action (e.g. lock, open settings sheet). */
    action?: () => void;
}

// Linear-style command palette. Phase 2 ships zone navigation + legacy-
// screen jumps + Lock + Reindex commands. Phase 4 wires entity-level
// search (people/places/albums/trips); Phase 7 hooks it into the same
// universal-search resolver as the Search hub.
export function CommandPalette({
    open,
    onOpenChange,
    onOpenSettings,
    onOpenWizard,
}: CommandPaletteProps) {
    const setView = useSession((s) => s.setView);
    const reset = useSession((s) => s.reset);

    // Esc to close, Cmd-K to toggle. Always wired up — the parent only
    // controls `open`, so the listener can't double-fire.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if ((e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                onOpenChange(!open);
            }
            if (e.key === "Escape" && open) {
                onOpenChange(false);
            }
        };
        window.addEventListener("keydown", onKey);
        return () => window.removeEventListener("keydown", onKey);
    }, [open, onOpenChange]);

    if (!open) return null;

    const goto = (view: View) => {
        setView(view);
        onOpenChange(false);
    };

    const lock = async () => {
        await api.lock();
        reset();
        onOpenChange(false);
    };

    const reindex = async () => {
        try {
            await api.mlReindex();
        } catch {
            // Silent — Phase 8's toast system will surface the error.
        }
        onOpenChange(false);
    };

    const zoneCommands: NavCommand[] = [
        {
            id: "go-library",
            label: "Library",
            icon: <Images size={16} />,
            keywords: "library timeline photos grid",
            view: { kind: "library" },
        },
        {
            id: "go-for-you",
            label: "For You",
            icon: <Sparkles size={16} />,
            keywords: "for you home memories featured",
            view: { kind: "for-you" },
        },
        {
            id: "go-albums",
            label: "Albums",
            icon: <FolderOpen size={16} />,
            keywords: "albums collections",
            view: { kind: "albums" },
        },
        {
            id: "go-search",
            label: "Search",
            icon: <Search size={16} />,
            keywords: "search find query",
            view: { kind: "search" },
        },
    ];

    const browseCommands: NavCommand[] = [
        {
            id: "go-people",
            label: "People",
            icon: <Users size={16} />,
            keywords: "people faces persons",
            view: { kind: "people" },
        },
        {
            id: "go-pets",
            label: "Pets",
            icon: <PawPrint size={16} />,
            keywords: "pets animals dog cat",
            view: { kind: "pets" },
        },
        {
            id: "go-trips",
            label: "Trips",
            icon: <Plane size={16} />,
            keywords: "trips travel vacations",
            view: { kind: "trips" },
        },
        {
            id: "go-map",
            label: "Map",
            icon: <MapPin size={16} />,
            keywords: "map places locations gps",
            view: { kind: "map" },
        },
        {
            id: "go-smart",
            label: "Smart albums",
            icon: <Star size={16} />,
            keywords: "smart albums rules auto",
            view: { kind: "smart_albums" },
        },
        {
            id: "go-duplicates",
            label: "Duplicates",
            icon: <Copy size={16} />,
            keywords: "duplicates dedupe near",
            view: { kind: "duplicates" },
        },
        {
            id: "go-memories",
            label: "Memories",
            icon: <Camera size={16} />,
            keywords: "memories on this day",
            view: { kind: "memories" },
        },
    ];

    const actionCommands: NavCommand[] = [
        {
            id: "open-settings",
            label: "Open settings",
            icon: <Settings size={16} />,
            keywords: "settings preferences sources peers ml",
            action: () => {
                onOpenSettings();
                onOpenChange(false);
            },
        },
        {
            id: "open-ml-wizard",
            label: "Manage on-device AI",
            icon: <Sparkles size={16} />,
            keywords: "ml ai models download wizard runtime",
            action: () => {
                onOpenWizard();
                onOpenChange(false);
            },
        },
        {
            id: "reindex-ml",
            label: "Reindex ML",
            icon: <RefreshCcw size={16} />,
            keywords: "reindex ml jobs faces clip embed detect",
            action: reindex,
        },
        {
            id: "lock-vault",
            label: "Lock vault",
            icon: <Lock size={16} />,
            keywords: "lock logout exit close",
            action: lock,
        },
    ];

    return (
        <div className="kp-cmdk-overlay" onClick={() => onOpenChange(false)}>
            <Command
                className="kp-cmdk-panel"
                onClick={(e) => e.stopPropagation()}
                label="Quick search"
            >
                <Command.Input
                    placeholder="Search Keepsake or jump to a view…"
                    className="kp-cmdk-input"
                    autoFocus
                />
                <Command.List className="kp-cmdk-list">
                    <Command.Empty className="kp-cmdk-empty">
                        No matches.
                    </Command.Empty>

                    <Command.Group heading="Go to" className="kp-cmdk-group">
                        {zoneCommands.map((c) => (
                            <CommandItem
                                key={c.id}
                                command={c}
                                goto={goto}
                            />
                        ))}
                    </Command.Group>

                    <Command.Group heading="Browse" className="kp-cmdk-group">
                        {browseCommands.map((c) => (
                            <CommandItem
                                key={c.id}
                                command={c}
                                goto={goto}
                            />
                        ))}
                    </Command.Group>

                    <Command.Group heading="Actions" className="kp-cmdk-group">
                        {actionCommands.map((c) => (
                            <CommandItem
                                key={c.id}
                                command={c}
                                goto={goto}
                            />
                        ))}
                    </Command.Group>
                </Command.List>
            </Command>
        </div>
    );
}

function CommandItem({
    command,
    goto,
}: {
    command: NavCommand;
    goto: (v: View) => void;
}) {
    return (
        <Command.Item
            value={`${command.label} ${command.keywords}`}
            className="kp-cmdk-item"
            onSelect={() => {
                if (command.view) goto(command.view);
                else if (command.action) command.action();
            }}
        >
            {command.icon}
            <span>{command.label}</span>
        </Command.Item>
    );
}
