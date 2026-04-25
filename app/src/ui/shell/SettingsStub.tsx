import { ChevronRight, FolderInput, Share2, Sparkles } from "lucide-react";
import { Sheet } from "../../components";
import { useSession } from "../../state/session";

interface SettingsStubProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onOpenWizard: () => void;
}

// Phase 2 placeholder. Phase 8 replaces this with the full Settings sheet
// (Appearance, Sources, Peers, ML, Vault, About). For now it's a launcher
// that routes to the existing Sources / Peers screens and the ML wizard.
export function SettingsStub({
    open,
    onOpenChange,
    onOpenWizard,
}: SettingsStubProps) {
    const setView = useSession((s) => s.setView);

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
            description="Phase 2 stub — full settings sheet lands in Phase 8."
        >
            <div className="kp-settings-stub">
                <button
                    type="button"
                    className="kp-settings-stub-row"
                    onClick={() => goto({ kind: "sources" })}
                >
                    <FolderInput size={20} />
                    <div>
                        <strong>Sources</strong>
                        <small>Folders the library imports from</small>
                    </div>
                    <ChevronRight size={16} />
                </button>
                <button
                    type="button"
                    className="kp-settings-stub-row"
                    onClick={() => goto({ kind: "peers" })}
                >
                    <Share2 size={20} />
                    <div>
                        <strong>Peers</strong>
                        <small>Pairing tickets and incoming shares</small>
                    </div>
                    <ChevronRight size={16} />
                </button>
                <button
                    type="button"
                    className="kp-settings-stub-row"
                    onClick={() => {
                        onOpenChange(false);
                        onOpenWizard();
                    }}
                >
                    <Sparkles size={20} />
                    <div>
                        <strong>ML &amp; on-device AI</strong>
                        <small>Models, runtime, reindex</small>
                    </div>
                    <ChevronRight size={16} />
                </button>
            </div>
        </Sheet>
    );
}
