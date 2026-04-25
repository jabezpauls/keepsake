import { useState } from "react";
import {
    Camera,
    Heart,
    Plus,
    Settings,
    Share2,
    Sparkles,
    Trash2,
} from "lucide-react";
import {
    Avatar,
    Breadcrumb,
    Button,
    Card,
    Chip,
    Confirm,
    DropdownItem,
    DropdownMenu,
    DropdownSeparator,
    EmptyState,
    EntityChip,
    IconButton,
    Modal,
    Popover,
    Sheet,
    Skeleton,
    Spinner,
    Tabs,
    TabsContent,
    TabsList,
    TabsTrigger,
    ToastProvider,
    Tooltip,
    TooltipProvider,
    useToast,
} from "./index";

/*
 * Dev-only primitive gallery. Mounted by main.tsx when the URL contains
 * `?gallery=1` and import.meta.env.DEV is true. Lets us eyeball every
 * primitive in light + dark + reduced-motion modes without spinning up
 * the rest of the app or unlocking a vault.
 *
 * This file is the visual snapshot target for Phase 1's verification:
 * Playwright captures screenshots of each section in both themes, and
 * subsequent phases compare against the baseline.
 */

type Theme = "auto" | "light" | "dark";

function ThemeToggle({ theme, onChange }: { theme: Theme; onChange: (t: Theme) => void }) {
    return (
        <div className="kp-row" role="radiogroup" aria-label="Theme">
            {(["auto", "light", "dark"] as Theme[]).map((t) => (
                <Chip
                    key={t}
                    active={theme === t}
                    onClick={() => onChange(t)}
                    size="sm"
                >
                    {t}
                </Chip>
            ))}
        </div>
    );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
    return (
        <section style={{ marginBottom: "var(--space-9)" }}>
            <h2
                style={{
                    font: "var(--font-title-1)",
                    margin: "0 0 var(--space-4) 0",
                }}
            >
                {title}
            </h2>
            <div className="kp-stack">{children}</div>
        </section>
    );
}

function Row({ children }: { children: React.ReactNode }) {
    return <div className="kp-row" style={{ flexWrap: "wrap" }}>{children}</div>;
}

function ToastDemo() {
    const { toast } = useToast();
    return (
        <Row>
            <Button onClick={() => toast({ title: "Saved", variant: "success" })}>
                Success toast
            </Button>
            <Button
                onClick={() =>
                    toast({
                        title: "Reindex queued",
                        description: "47 jobs",
                        variant: "default",
                    })
                }
            >
                Default toast
            </Button>
            <Button
                onClick={() =>
                    toast({
                        title: "Public link revoked",
                        variant: "warning",
                    })
                }
            >
                Warning toast
            </Button>
            <Button
                onClick={() =>
                    toast({
                        title: "Failed to share",
                        description: "Peer unreachable",
                        variant: "danger",
                        action: { label: "Retry", onClick: () => undefined },
                    })
                }
            >
                Danger toast (with action)
            </Button>
        </Row>
    );
}

export default function Gallery() {
    const [theme, setTheme] = useState<Theme>("auto");
    const [modalOpen, setModalOpen] = useState(false);
    const [sheetOpen, setSheetOpen] = useState(false);
    const [confirmOpen, setConfirmOpen] = useState(false);
    const [tabValue, setTabValue] = useState("photos");
    const [chipState, setChipState] = useState<Record<string, boolean>>({
        video: false,
        raw: true,
        screenshot: false,
        live: false,
    });

    // Apply theme to <html data-theme="…">. Removing the attribute when
    // theme === "auto" lets the prefers-color-scheme media query take over.
    if (typeof document !== "undefined") {
        const root = document.documentElement;
        if (theme === "auto") root.removeAttribute("data-theme");
        else root.setAttribute("data-theme", theme);
    }

    return (
        <TooltipProvider>
            <ToastProvider>
                <div
                    style={{
                        padding: "var(--space-6)",
                        background: "var(--color-canvas)",
                        minHeight: "100vh",
                    }}
                >
                    <header
                        style={{
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "space-between",
                            marginBottom: "var(--space-7)",
                        }}
                    >
                        <h1 style={{ font: "var(--font-display)", margin: 0 }}>
                            Keepsake design gallery
                        </h1>
                        <ThemeToggle theme={theme} onChange={setTheme} />
                    </header>

                    <p
                        style={{
                            color: "var(--color-text-secondary)",
                            font: "var(--font-body)",
                            maxWidth: "60ch",
                            marginBottom: "var(--space-7)",
                        }}
                    >
                        Phase 1 primitives in light + dark + reduced-motion.
                        Toggle the theme above. Add{" "}
                        <code className="kp-mono">?gallery=1</code> to the
                        Vite preview URL to reach this page in any environment.
                    </p>

                    <Section title="Buttons">
                        <Row>
                            <Button variant="primary">Primary</Button>
                            <Button variant="secondary">Secondary</Button>
                            <Button variant="ghost">Ghost</Button>
                            <Button variant="danger">Danger</Button>
                        </Row>
                        <Row>
                            <Button size="sm">Small</Button>
                            <Button size="md">Medium</Button>
                            <Button size="lg">Large</Button>
                        </Row>
                        <Row>
                            <Button leadingIcon={<Plus size={14} />}>New album</Button>
                            <Button
                                trailingIcon={<Share2 size={14} />}
                                variant="primary"
                            >
                                Share
                            </Button>
                            <Button loading>Saving</Button>
                            <Button disabled>Disabled</Button>
                        </Row>
                        <Row>
                            <IconButton
                                icon={<Heart size={16} />}
                                label="Favorite"
                            />
                            <IconButton
                                icon={<Settings size={16} />}
                                label="Settings"
                                size="lg"
                            />
                            <IconButton
                                icon={<Trash2 size={16} />}
                                label="Delete"
                                size="sm"
                            />
                            <IconButton
                                icon={<Sparkles size={16} />}
                                label="ML status"
                                active
                            />
                        </Row>
                    </Section>

                    <Section title="Chips">
                        <Row>
                            {Object.entries(chipState).map(([key, active]) => (
                                <Chip
                                    key={key}
                                    active={active}
                                    onClick={() =>
                                        setChipState((s) => ({
                                            ...s,
                                            [key]: !s[key],
                                        }))
                                    }
                                >
                                    {key}
                                </Chip>
                            ))}
                        </Row>
                        <Row>
                            <Chip size="sm">small</Chip>
                            <Chip size="md">medium</Chip>
                            <Chip size="lg">large</Chip>
                        </Row>
                    </Section>

                    <Section title="Entity chips (the ecosystem connector)">
                        <Row>
                            <EntityChip
                                entity={{
                                    kind: "place",
                                    placeId: "JP:tokyo",
                                    name: "Tokyo, Japan",
                                }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{
                                    kind: "date",
                                    utcDay: 19500,
                                    label: "March 2024",
                                    precision: "month",
                                }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{
                                    kind: "trip",
                                    id: 1,
                                    name: "Italy 2024",
                                }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{
                                    kind: "album",
                                    id: 1,
                                    name: "Family",
                                }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{
                                    kind: "peer",
                                    nodeIdHex: "abcd1234567890abcdef",
                                }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{ kind: "camera", make: "SONY ILCE-7M4" }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{ kind: "lens", lens: "FE 24-70mm F2.8" }}
                                onClick={() => undefined}
                            />
                            <EntityChip
                                entity={{
                                    kind: "category",
                                    key: "raw",
                                    label: "RAW",
                                }}
                                onClick={() => undefined}
                            />
                        </Row>
                    </Section>

                    <Section title="Cards">
                        <Row>
                            <Card hoverable style={{ width: 200 }}>
                                <h3 style={{ margin: 0, font: "var(--font-title-2)" }}>
                                    Italy 2024
                                </h3>
                                <p
                                    style={{
                                        margin: "var(--space-2) 0 0 0",
                                        color: "var(--color-text-secondary)",
                                        font: "var(--font-caption)",
                                    }}
                                >
                                    142 photos
                                </p>
                            </Card>
                            <Card padding="lg" style={{ width: 240 }}>
                                <h3 style={{ margin: 0, font: "var(--font-title-2)" }}>
                                    Settings
                                </h3>
                                <p
                                    style={{
                                        margin: "var(--space-2) 0 0 0",
                                        color: "var(--color-text-secondary)",
                                        font: "var(--font-caption)",
                                    }}
                                >
                                    Static card without hover
                                </p>
                            </Card>
                        </Row>
                    </Section>

                    <Section title="Avatars">
                        <Row>
                            <Avatar size="xs" fallback="JM" />
                            <Avatar size="sm" fallback="JM" />
                            <Avatar size="md" fallback="JM" />
                            <Avatar size="lg" fallback="JM" />
                            <Avatar size="xl" fallback="JM" />
                        </Row>
                    </Section>

                    <Section title="Loading + empty states">
                        <Row>
                            <Skeleton width={200} height={120} />
                            <Skeleton width={120} height={120} radius="50%" />
                            <Spinner />
                        </Row>
                        <Card padding="none" style={{ width: 480 }}>
                            <EmptyState
                                icon={<Camera size={32} />}
                                title="No photos yet"
                                hint="Add a source from Settings to start your library."
                                actions={
                                    <Button variant="primary" leadingIcon={<Plus size={14} />}>
                                        Add source
                                    </Button>
                                }
                            />
                        </Card>
                    </Section>

                    <Section title="Tabs">
                        <Tabs value={tabValue} onValueChange={setTabValue}>
                            <TabsList>
                                <TabsTrigger value="photos">Photos</TabsTrigger>
                                <TabsTrigger value="people">People</TabsTrigger>
                                <TabsTrigger value="places">Places</TabsTrigger>
                            </TabsList>
                            <TabsContent value="photos">Photos panel.</TabsContent>
                            <TabsContent value="people">People panel.</TabsContent>
                            <TabsContent value="places">Places panel.</TabsContent>
                        </Tabs>
                    </Section>

                    <Section title="Overlays">
                        <Row>
                            <Button onClick={() => setModalOpen(true)}>Open Modal</Button>
                            <Button onClick={() => setSheetOpen(true)}>Open Sheet</Button>
                            <Button
                                variant="danger"
                                onClick={() => setConfirmOpen(true)}
                            >
                                Open Confirm
                            </Button>
                            <Tooltip content="This is a tooltip">
                                <Button variant="ghost">Hover for tooltip</Button>
                            </Tooltip>
                            <Popover
                                trigger={<Button variant="ghost">Click for popover</Button>}
                            >
                                <p style={{ margin: 0 }}>
                                    Popover content with rich children.
                                </p>
                            </Popover>
                            <DropdownMenu trigger={<Button>Open dropdown</Button>}>
                                <DropdownItem onSelect={() => undefined}>
                                    Share
                                </DropdownItem>
                                <DropdownItem onSelect={() => undefined}>
                                    Export
                                </DropdownItem>
                                <DropdownSeparator />
                                <DropdownItem
                                    variant="danger"
                                    onSelect={() => undefined}
                                >
                                    Delete
                                </DropdownItem>
                            </DropdownMenu>
                        </Row>
                    </Section>

                    <Section title="Toasts">
                        <ToastDemo />
                    </Section>

                    <Section title="Breadcrumbs">
                        <Breadcrumb
                            items={[
                                { label: "Library", onClick: () => undefined },
                                { label: "Tokyo, Japan", onClick: () => undefined },
                                { label: "IMG_4729.heic" },
                            ]}
                        />
                    </Section>

                    <Modal
                        open={modalOpen}
                        onOpenChange={setModalOpen}
                        title="Modal title"
                        description="Centered overlay with a backdrop. Esc to close."
                        actions={
                            <>
                                <Button variant="ghost" onClick={() => setModalOpen(false)}>
                                    Cancel
                                </Button>
                                <Button
                                    variant="primary"
                                    onClick={() => setModalOpen(false)}
                                >
                                    OK
                                </Button>
                            </>
                        }
                    >
                        <p>Modal body content.</p>
                    </Modal>

                    <Sheet
                        open={sheetOpen}
                        onOpenChange={setSheetOpen}
                        side="right"
                        title="Sheet title"
                        description="Side-anchored panel. Slides in from the right."
                    >
                        <p style={{ font: "var(--font-body)" }}>
                            Use sheets for settings, share dialogs, info pane.
                        </p>
                    </Sheet>

                    <Confirm
                        open={confirmOpen}
                        onOpenChange={setConfirmOpen}
                        title="Delete this album?"
                        description="The photos remain in your library. The album itself is removed."
                        variant="danger"
                        confirmLabel="Delete album"
                        onConfirm={() => undefined}
                    />
                </div>
            </ToastProvider>
        </TooltipProvider>
    );
}
