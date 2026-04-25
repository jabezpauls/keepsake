// Theme + motion persistence — kept in its own file so SettingsSheet
// can stay a clean component-only export (eslint-plugin-react-refresh).
//
// Storage layout (localStorage):
//   mv-theme  ∈ "auto" | "light" | "dark"
//   mv-motion ∈ "auto" | "full" | "reduced"
//
// At paint time we set:
//   :root[data-theme]   to the explicit choice (omitted for "auto", so
//                       the prefers-color-scheme media query takes over)
//   :root[data-motion]  to "reduced" / "full" overrides — see
//                       settings.css for the cascade rules
//
// The bootstrap call lives in main.tsx so the first paint already wears
// the user's choice — avoids a flash of the wrong theme.

export type Theme = "auto" | "light" | "dark";
export type Motion = "auto" | "full" | "reduced";

const THEME_KEY = "mv-theme";
const MOTION_KEY = "mv-motion";

export function readTheme(): Theme {
    try {
        const v = localStorage.getItem(THEME_KEY);
        if (v === "light" || v === "dark" || v === "auto") return v;
    } catch {
        // private mode, quota
    }
    return "auto";
}

export function readMotion(): Motion {
    try {
        const v = localStorage.getItem(MOTION_KEY);
        if (v === "full" || v === "reduced" || v === "auto") return v;
    } catch {
        // private mode, quota
    }
    return "auto";
}

export function applyTheme(theme: Theme) {
    if (typeof document === "undefined") return;
    const root = document.documentElement;
    if (theme === "auto") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
    try {
        localStorage.setItem(THEME_KEY, theme);
    } catch {
        // ignore
    }
}

export function applyMotion(motion: Motion) {
    if (typeof document === "undefined") return;
    const root = document.documentElement;
    if (motion === "auto") {
        root.removeAttribute("data-motion");
    } else if (motion === "reduced") {
        root.setAttribute("data-motion", "reduced");
    } else {
        root.setAttribute("data-motion", "full");
    }
    try {
        localStorage.setItem(MOTION_KEY, motion);
    } catch {
        // ignore
    }
}

export function bootstrapAppearance() {
    applyTheme(readTheme());
    applyMotion(readMotion());
}
