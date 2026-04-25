import * as RadixToast from "@radix-ui/react-toast";
import { ReactNode, useCallback, useMemo, useState } from "react";
import { cn } from "./cn";
import { ToastContext, ToastOptions } from "./useToast";

interface ToastInstance extends ToastOptions {
    id: number;
}

interface ToastProviderProps {
    children: ReactNode;
}

// Mount once at the root of the app (App.tsx). Children call useToast()
// (re-exported from "./useToast") to fire toasts; the provider holds the
// queue and renders the viewport in a portal at the bottom-right.
export function ToastProvider({ children }: ToastProviderProps) {
    const [toasts, setToasts] = useState<ToastInstance[]>([]);

    const toast = useCallback((options: ToastOptions) => {
        setToasts((prev) => [...prev, { id: Date.now() + Math.random(), ...options }]);
    }, []);

    const dismiss = useCallback((id: number) => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
    }, []);

    const api = useMemo(() => ({ toast }), [toast]);

    return (
        <ToastContext.Provider value={api}>
            <RadixToast.Provider swipeDirection="right">
                {children}
                {toasts.map((t) => (
                    <RadixToast.Root
                        key={t.id}
                        duration={t.duration ?? 4000}
                        onOpenChange={(open) => {
                            if (!open) dismiss(t.id);
                        }}
                        className={cn("kp-toast")}
                        data-variant={t.variant ?? "default"}
                    >
                        {t.title && (
                            <RadixToast.Title className="kp-toast-title">
                                {t.title}
                            </RadixToast.Title>
                        )}
                        {t.description && (
                            <RadixToast.Description className="kp-toast-description">
                                {t.description}
                            </RadixToast.Description>
                        )}
                        {t.action && (
                            <RadixToast.Action
                                altText={t.action.label}
                                className="kp-toast-action"
                                onClick={t.action.onClick}
                            >
                                {t.action.label}
                            </RadixToast.Action>
                        )}
                    </RadixToast.Root>
                ))}
                <RadixToast.Viewport className="kp-toast-viewport" />
            </RadixToast.Provider>
        </ToastContext.Provider>
    );
}
