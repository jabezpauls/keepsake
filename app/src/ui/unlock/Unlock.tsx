import { useEffect, useRef, useState } from "react";
import { api } from "../../ipc";
import { useSession } from "../../state/session";

// The unlock screen serves three modes:
//   1. create-user (first run — no vault exists yet)
//   2. unlock       (returning user — vault exists)
//   3. hidden       (long-press on the logo while at the unlock screen)
//
// Hidden-mode failures look IDENTICAL to main-unlock failures (§9
// plausible-deniability) — same shake animation, same visible state reset.

type Mode = "create" | "unlock" | "hidden";

export default function Unlock() {
    const [mode, setMode] = useState<Mode>("unlock");
    const [username, setUsername] = useState("");
    const [password, setPassword] = useState("");
    const [password2, setPassword2] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const [shake, setShake] = useState(false);
    const setSession = useSession((s) => s.setSession);
    const setHiddenUnlocked = useSession((s) => s.setHiddenUnlocked);

    // Determine create vs. unlock on mount.
    useEffect(() => {
        void (async () => {
            try {
                const exists = await api.userExists();
                setMode(exists ? "unlock" : "create");
            } catch {
                // On any error, default to unlock and let user retry.
                setMode("unlock");
            }
        })();
    }, []);

    const triggerShake = () => {
        setShake(true);
        setTimeout(() => setShake(false), 400);
    };

    const onSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        setError(null);
        setBusy(true);
        try {
            if (mode === "create") {
                if (password !== password2) {
                    setError("passwords do not match");
                    triggerShake();
                    return;
                }
                const session = await api.createUser(username, password);
                setSession(session);
            } else if (mode === "unlock") {
                const session = await api.unlock(username, password);
                setSession(session);
            } else if (mode === "hidden") {
                // Hidden vault unlock is attempted against a main-unlocked
                // session. For plausible-deniability the UI returns to the
                // unlock screen with the same shake regardless of outcome.
                const session = await api.unlock(username, password);
                setSession(session);
                const ok = await api.unlockHidden(password2);
                if (ok) setHiddenUnlocked(true);
            }
        } catch (err) {
            setError(String(err));
            triggerShake();
            setPassword("");
            setPassword2("");
        } finally {
            setBusy(false);
        }
    };

    // Long-press gesture on the logo enters hidden mode.
    const pressTimer = useRef<number | null>(null);
    const onLogoPressStart = () => {
        pressTimer.current = window.setTimeout(() => {
            setMode("hidden");
            setError(null);
            setPassword("");
            setPassword2("");
        }, 2000);
    };
    const onLogoPressEnd = () => {
        if (pressTimer.current !== null) {
            window.clearTimeout(pressTimer.current);
            pressTimer.current = null;
        }
    };

    return (
        <main className={`unlock ${shake ? "shake" : ""}`.trim()}>
            <h1
                className="logo"
                onMouseDown={onLogoPressStart}
                onMouseUp={onLogoPressEnd}
                onMouseLeave={onLogoPressEnd}
                onTouchStart={onLogoPressStart}
                onTouchEnd={onLogoPressEnd}
            >
                Media Vault
            </h1>

            <form onSubmit={onSubmit} className="unlock-form">
                <label>
                    <span>Username</span>
                    <input
                        type="text"
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        autoComplete="username"
                        required
                        disabled={busy}
                    />
                </label>
                <label>
                    <span>Password</span>
                    <input
                        type="password"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        autoComplete={mode === "create" ? "new-password" : "current-password"}
                        required
                        disabled={busy}
                    />
                </label>
                {(mode === "create" || mode === "hidden") && (
                    <label>
                        <span>{mode === "hidden" ? "Hidden-vault password" : "Confirm password"}</span>
                        <input
                            type="password"
                            value={password2}
                            onChange={(e) => setPassword2(e.target.value)}
                            autoComplete="new-password"
                            required
                            disabled={busy}
                        />
                    </label>
                )}
                <button type="submit" disabled={busy}>
                    {mode === "create" ? "Create vault" : "Unlock"}
                </button>
                {error && <p className="error">{error}</p>}
            </form>

            {mode === "hidden" && (
                <p className="hint">Returning to main unlock cancels hidden mode.</p>
            )}
        </main>
    );
}
