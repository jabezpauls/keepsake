import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";

// Fonts — Inter Variable for UI, JetBrains Mono for monospace blocks.
// Tauri is offline-first, so fonts are bundled rather than fetched.
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";

// Design system: tokens (theme-aware CSS variables) ↦ primitives (base
// component styles) ↦ legacy global stylesheet (still used by every
// pre-redesign screen until Phase 9). Order matters: tokens before
// primitives before legacy, so each layer can reference the previous.
import "./styles/tokens.css";
import "./components/primitives.css";
import "./styles.css";

// Apply the user's persisted theme + motion choices before React
// hydrates, so the first paint already wears the right colour scheme
// (no flash of unstyled / wrong-themed content).
import { bootstrapAppearance } from "./ui/settings/appearance";
bootstrapAppearance();

const queryClient = new QueryClient();

// Dev-only design-system gallery. Mount with `?gallery=1` to inspect
// every primitive in light + dark + reduced-motion. Tree-shaken from
// production builds because import.meta.env.DEV is statically false.
async function bootstrap(): Promise<React.ReactElement> {
    if (
        import.meta.env.DEV &&
        new URLSearchParams(window.location.search).get("gallery") === "1"
    ) {
        const { default: Gallery } = await import("./components/_gallery");
        return <Gallery />;
    }
    return (
        <QueryClientProvider client={queryClient}>
            <App />
        </QueryClientProvider>
    );
}

void bootstrap().then((tree) =>
    ReactDOM.createRoot(document.getElementById("root")!).render(
        <React.StrictMode>{tree}</React.StrictMode>,
    ),
);
