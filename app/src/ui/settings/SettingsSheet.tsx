import { useEffect, useState } from "react";
import {
    ChevronRight,
    FolderInput,
    Info,
    Lock,
    Moon,
    Share2,
    Sparkles,
    Sun,
    SunMoon,
} from "lucide-react";
import { Button, Sheet } from "../../components";
import { useSession } from "../../state/session";
import {
    applyMotion,
    applyTheme,
    readMotion,
    readTheme,
    type Motion,
    type Theme,
} from "./appearance";
import "./settings.css";

interface SettingsSheetProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onOpenWizard: () => void;
}

// Phase 8 SettingsSheet — replaces the Phase 2 stub. Single sheet
// hosting:
//
//   * Appearance — theme + reduced-motion overrides (writes the
//     :root[data-theme] / :root[data-motion] attributes; tokens.css
//     swaps values without a re-render).
//   * Sources / Peers — link to existing screens (Phase 9 polish may
//     embed them inline).
//   * ML — link to the wizard.
//   * Vault — Lock + reset.
//   * About — version + license.
//
// Settings persist in localStorage so the choice survives reloads.
export function SettingsSheet({
    open,
    onOpenChange,
    onOpenWizard,
}: SettingsSheetProps) {
    const setView = useSession((s) => s.setView);
    const reset = useSession((s) => s.reset);

    const [theme, setTheme] = useState<Theme>(() => readTheme());
    const [motion, setMotion] = useState<Motion>(() => readMotion());

    useEffect(() => {
        applyTheme(theme);
    }, [theme]);

    useEffect(() => {
        applyMotion(motion);
    }, [motion]);

    const goto = (view: Parameters<typeof setView>[0]) => {
        setView(view);
        onOpenChange(false);
    };

    return (
        <Sheet
            open={open}
            onOpenChange={onOpenChange}
            side="right"
            title="Settings"
        >
            <div className="kp-settings">
                <Section title="Appearance">
                    <FieldRow label="Theme" hint="Auto follows your OS preference.">
                        <SegmentedControl
                            value={theme}
                            onChange={setTheme}
                            options={[
                                { value: "auto", label: "Auto", icon: <SunMoon size={14} /> },
                                { value: "light", label: "Light", icon: <Sun size={14} /> },
                                { value: "dark", label: "Dark", icon: <Moon size={14} /> },
                            ]}
                        />
                    </FieldRow>
                    <FieldRow
                        label="Motion"
                        hint="Reduces shared-element transitions and slideshow Ken-Burns. Auto follows your OS prefers-reduced-motion setting."
                    >
                        <SegmentedControl
                            value={motion}
                            onChange={setMotion}
                            options={[
                                { value: "auto", label: "Auto" },
                                { value: "full", label: "Full" },
                                { value: "reduced", label: "Reduced" },
                            ]}
                        />
                    </FieldRow>
                </Section>

                <Section title="Library">
                    <Row
                        icon={<FolderInput size={20} />}
                        title="Sources"
                        hint="Folders the library imports from"
                        onClick={() => goto({ kind: "sources" })}
                    />
                    <Row
                        icon={<Share2 size={20} />}
                        title="Peers"
                        hint="Pairing tickets and incoming shares"
                        onClick={() => goto({ kind: "peers" })}
                    />
                    <Row
                        icon={<Sparkles size={20} />}
                        title="ML & on-device AI"
                        hint="Models, runtime, reindex"
                        onClick={() => {
                            onOpenChange(false);
                            onOpenWizard();
                        }}
                    />
                </Section>

                <Section title="Vault">
                    <Row
                        icon={<Lock size={20} />}
                        title="Lock vault"
                        hint="Sign out and require the password again"
                        onClick={async () => {
                            try {
                                const { api } = await import("../../ipc");
                                await api.lock();
                            } finally {
                                reset();
                                onOpenChange(false);
                            }
                        }}
                    />
                </Section>

                <Section title="About">
                    <p className="kp-settings-about">
                        <Info size={14} aria-hidden /> Keepsake — local-first
                        encrypted media library. AGPL-3.0.
                    </p>
                </Section>
            </div>
        </Sheet>
    );
}

function Section({
    title,
    children,
}: {
    title: string;
    children: React.ReactNode;
}) {
    return (
        <section className="kp-settings-section">
            <h3>{title}</h3>
            {children}
        </section>
    );
}

function FieldRow({
    label,
    hint,
    children,
}: {
    label: string;
    hint?: string;
    children: React.ReactNode;
}) {
    return (
        <div className="kp-settings-field">
            <div className="kp-settings-field-label">
                <strong>{label}</strong>
                {hint && <span>{hint}</span>}
            </div>
            <div className="kp-settings-field-value">{children}</div>
        </div>
    );
}

interface RowProps {
    icon: React.ReactNode;
    title: string;
    hint: string;
    onClick: () => void;
}

function Row({ icon, title, hint, onClick }: RowProps) {
    return (
        <button type="button" className="kp-settings-row" onClick={onClick}>
            <div className="kp-settings-row-icon">{icon}</div>
            <div className="kp-settings-row-meta">
                <strong>{title}</strong>
                <span>{hint}</span>
            </div>
            <ChevronRight size={16} />
        </button>
    );
}

interface SegmentedControlProps<T extends string> {
    value: T;
    onChange: (v: T) => void;
    options: { value: T; label: string; icon?: React.ReactNode }[];
}

function SegmentedControl<T extends string>({
    value,
    onChange,
    options,
}: SegmentedControlProps<T>) {
    return (
        <div className="kp-segmented">
            {options.map((o) => (
                <Button
                    key={o.value}
                    variant={value === o.value ? "primary" : "ghost"}
                    size="sm"
                    leadingIcon={o.icon}
                    onClick={() => onChange(o.value)}
                >
                    {o.label}
                </Button>
            ))}
        </div>
    );
}

