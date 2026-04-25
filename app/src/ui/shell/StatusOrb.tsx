import { useEffect, useState } from "react";
import { api } from "../../ipc";
import type { MlStatus } from "../../bindings/MlStatus";
import { Tooltip } from "../../components";

interface StatusOrbProps {
    onClick?: () => void;
}

// 8 px circular ML status indicator.
//   gray            — ML compiled out (build without --features ml-models)
//   amber-pulsing   — models needed (clickable → opens wizard)
//   green-steady    — idle
//   green-pulsing   — running jobs
//   red             — failures > 0
//
// Hover → tooltip with detailed status. Click → opens the ML wizard
// (or settings → ML when Phase 8 lands the unified sheet).
export function StatusOrb({ onClick }: StatusOrbProps) {
    const [ml, setMl] = useState<MlStatus | null>(null);

    useEffect(() => {
        let alive = true;
        const tick = async () => {
            try {
                const s = await api.mlStatus();
                if (alive) setMl(s);
            } catch {
                // Ignore — keep last known value.
            }
        };
        void tick();
        const h = window.setInterval(tick, 4000);
        return () => {
            alive = false;
            window.clearInterval(h);
        };
    }, []);

    if (!ml) return null;

    const { state, label } = orbState(ml);
    return (
        <Tooltip content={label}>
            <button
                type="button"
                className="kp-status-orb"
                data-state={state}
                onClick={onClick}
                aria-label={`ML status: ${label}`}
            />
        </Tooltip>
    );
}

interface OrbState {
    state: "off" | "missing" | "idle" | "running" | "failed";
    label: string;
}

function orbState(ml: MlStatus): OrbState {
    if (!ml.models_available) {
        return { state: "off", label: "ML off (compiled without ml-models)" };
    }
    if (!ml.runtime_loaded) {
        return {
            state: "missing",
            label: "Models needed — click to download (~2 GB)",
        };
    }
    if (ml.failed > 0) {
        return {
            state: "failed",
            label: `${ml.failed} failed jobs · ${ml.pending} pending · ${ml.running} running`,
        };
    }
    const queued = ml.pending + ml.running;
    if (queued > 0) {
        return {
            state: "running",
            label: `${ml.execution_provider} · ${ml.pending} pending · ${ml.running} running`,
        };
    }
    return {
        state: "idle",
        label: `${ml.execution_provider} · idle · ${ml.done} done`,
    };
}
