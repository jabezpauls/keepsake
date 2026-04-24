import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../../ipc";
import type { DownloadEvent } from "../../bindings/DownloadEvent";
import type { ModelsStatus } from "../../bindings/ModelsStatus";

interface Props {
    onClose: () => void;
}

type PerFile = {
    name: string;
    downloaded: number;
    total: number;
    phase: "idle" | "downloading" | "verified" | "failed";
    reason?: string;
};

/**
 * First-run model download wizard. Invoked from the ML badge when the
 * runtime reports `!runtime_loaded` (weights missing). Streams live
 * progress from the `ml-download-event` channel and reloads the
 * runtime on successful completion so the badge flips from
 * "no weights" to the actual execution provider without a lock cycle.
 */
export default function ModelDownloadWizard({ onClose }: Props) {
    const queryClient = useQueryClient();
    const status = useQuery<ModelsStatus>({
        queryKey: ["ml-models-status"],
        queryFn: api.mlModelsStatus,
    });

    const [perFile, setPerFile] = useState<Record<string, PerFile>>({});
    const [phase, setPhase] = useState<"review" | "downloading" | "done" | "failed">("review");
    const [failedList, setFailedList] = useState<string[]>([]);

    // Live event stream from the downloader. Attached on mount, detached on
    // unmount so a re-opened wizard (post-failure retry) gets a fresh feed.
    useEffect(() => {
        let unsub: UnlistenFn | null = null;
        listen<DownloadEvent>("ml-download-event", (e) => {
            const ev = e.payload;
            setPerFile((prev) => {
                const next = { ...prev };
                if (ev.kind === "start") {
                    next[ev.name] = {
                        name: ev.name,
                        downloaded: 0,
                        total: ev.bytes_total,
                        phase: "downloading",
                    };
                } else if (ev.kind === "progress") {
                    next[ev.name] = {
                        name: ev.name,
                        downloaded: ev.bytes_downloaded,
                        total: Math.max(ev.bytes_total, ev.bytes_downloaded),
                        phase: "downloading",
                    };
                } else if (ev.kind === "verified") {
                    const row = prev[ev.name];
                    next[ev.name] = {
                        name: ev.name,
                        downloaded: row?.total ?? row?.downloaded ?? 0,
                        total: row?.total ?? 0,
                        phase: "verified",
                    };
                } else if (ev.kind === "file_failed") {
                    next[ev.name] = {
                        name: ev.name,
                        downloaded: prev[ev.name]?.downloaded ?? 0,
                        total: prev[ev.name]?.total ?? 0,
                        phase: "failed",
                        reason: ev.reason,
                    };
                }
                return next;
            });
            if (ev.kind === "all_done") {
                setFailedList(ev.failed);
                setPhase(ev.ok ? "done" : "failed");
                // Refresh badge + survey so the caller sees weights light up.
                queryClient.invalidateQueries({ queryKey: ["ml-status"] });
                queryClient.invalidateQueries({ queryKey: ["ml-models-status"] });
            }
        }).then((fn) => {
            unsub = fn;
        });
        return () => {
            unsub?.();
        };
    }, [queryClient]);

    const totals = useMemo(() => {
        const files = status.data?.files ?? [];
        const missing = files.filter((f) => !f.valid).length;
        const present = files.filter((f) => f.valid).length;
        return { missing, present, total: files.length };
    }, [status.data]);

    const start = async () => {
        setPhase("downloading");
        setPerFile({});
        setFailedList([]);
        try {
            await api.mlModelsDownload();
            // Best-effort runtime reload so the badge flips without a
            // lock/unlock. `ml_runtime_reload` is idempotent — re-runs the
            // bootstrap path and replaces the inner Arc on success.
            await api.mlRuntimeReload().catch(() => undefined);
        } catch (err) {
            // The terminal `all_done` event still sets `failed`; this catches
            // the outer error propagation path (e.g. models feature not built
            // in) and keeps the UI honest.
            setPhase("failed");
            setFailedList((prev) => (prev.length === 0 ? ["download command errored"] : prev));
            console.error("mlModelsDownload failed", err);
        }
    };

    const files = status.data?.files ?? [];

    return (
        <div className="share-modal-backdrop" onClick={onClose}>
            <div className="share-modal" onClick={(e) => e.stopPropagation()}>
                <header>
                    <h2>On-device AI models</h2>
                    <button className="close" onClick={onClose} aria-label="Close">
                        ×
                    </button>
                </header>

                <p style={{ color: "var(--muted)", margin: "0 0 0.75rem" }}>
                    Keepsake runs face recognition and semantic search on your
                    device. These ~2 GB of ONNX weights are downloaded once and
                    pinned by SHA-256. Nothing about your library ever leaves
                    the machine.
                </p>

                {phase === "review" && status.isLoading && <p>Checking…</p>}

                {phase === "review" && !status.isLoading && (
                    <>
                        <div className="ml-wizard-summary">
                            {totals.missing === 0 ? (
                                <span>All {totals.total} files present ✓</span>
                            ) : (
                                <span>
                                    {totals.missing} of {totals.total} files
                                    need to be downloaded
                                </span>
                            )}
                        </div>
                        <ul className="ml-wizard-filelist">
                            {files.map((f) => (
                                <li key={f.name}>
                                    <span>
                                        {f.valid ? "✓" : f.present ? "✗" : "·"}{" "}
                                        {f.name}
                                    </span>
                                    <span
                                        style={{
                                            color: "var(--muted)",
                                            fontSize: "0.85em",
                                        }}
                                    >
                                        {f.valid
                                            ? formatBytes(f.size_bytes)
                                            : f.present
                                              ? "stale"
                                              : "missing"}
                                    </span>
                                </li>
                            ))}
                        </ul>
                        <div className="share-modal-actions">
                            <button onClick={onClose}>Skip for now</button>
                            <button
                                className="primary"
                                onClick={start}
                                disabled={totals.missing === 0}
                            >
                                {totals.missing === 0
                                    ? "Nothing to download"
                                    : `Download ${totals.missing} file${totals.missing === 1 ? "" : "s"}`}
                            </button>
                        </div>
                    </>
                )}

                {(phase === "downloading" ||
                    phase === "done" ||
                    phase === "failed") && (
                    <>
                        <ul className="ml-wizard-filelist">
                            {files.map((f) => {
                                const row = perFile[f.name];
                                const glyph = row
                                    ? row.phase === "verified"
                                        ? "✓"
                                        : row.phase === "failed"
                                          ? "✗"
                                          : "↓"
                                    : f.valid
                                      ? "✓"
                                      : "·";
                                const pct = row?.total
                                    ? Math.min(
                                          100,
                                          Math.round(
                                              (row.downloaded / row.total) *
                                                  100,
                                          ),
                                      )
                                    : f.valid
                                      ? 100
                                      : 0;
                                return (
                                    <li key={f.name}>
                                        <div className="ml-wizard-row">
                                            <span>
                                                {glyph} {f.name}
                                            </span>
                                            <span
                                                style={{
                                                    color: "var(--muted)",
                                                    fontSize: "0.85em",
                                                }}
                                            >
                                                {row?.reason ??
                                                    (row?.total
                                                        ? `${formatBytes(row.downloaded)} / ${formatBytes(row.total)}`
                                                        : row?.phase ===
                                                            "verified"
                                                          ? "verified"
                                                          : "")}
                                            </span>
                                        </div>
                                        <div className="ml-wizard-bar">
                                            <div
                                                className="ml-wizard-bar-fill"
                                                style={{
                                                    width: `${pct}%`,
                                                }}
                                            />
                                        </div>
                                    </li>
                                );
                            })}
                        </ul>
                        {phase === "done" && (
                            <p style={{ color: "var(--good)" }}>
                                All set. The badge will switch to your active
                                execution provider shortly.
                            </p>
                        )}
                        {phase === "failed" && (
                            <p style={{ color: "var(--danger, crimson)" }}>
                                Download failed for:{" "}
                                {failedList.join(", ") || "one or more files"}.
                                Check your connection and try again.
                            </p>
                        )}
                        <div className="share-modal-actions">
                            {phase === "failed" && (
                                <button onClick={start}>Retry</button>
                            )}
                            <button
                                className="primary"
                                onClick={onClose}
                                disabled={phase === "downloading"}
                            >
                                {phase === "downloading" ? "Downloading…" : "Close"}
                            </button>
                        </div>
                    </>
                )}
            </div>
        </div>
    );
}

function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
