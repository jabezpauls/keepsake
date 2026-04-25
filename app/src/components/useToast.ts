import { createContext, ReactNode, useContext } from "react";

export type ToastVariant = "default" | "success" | "warning" | "danger";

export interface ToastOptions {
    title?: ReactNode;
    description?: ReactNode;
    variant?: ToastVariant;
    /** Auto-dismiss after N ms. Set to Infinity for sticky toasts. */
    duration?: number;
    action?: { label: string; onClick: () => void };
}

interface ToastApi {
    toast: (options: ToastOptions) => void;
}

// Context lives in its own module so that ToastProvider can be a clean
// component-only export — keeps eslint-plugin-react-refresh happy and
// avoids a roundtrip through the provider's barrel.
export const ToastContext = createContext<ToastApi | null>(null);

export function useToast(): ToastApi {
    const ctx = useContext(ToastContext);
    if (!ctx) throw new Error("useToast must be used within <ToastProvider>");
    return ctx;
}
