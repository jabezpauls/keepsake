// Tiny classname joiner. Falsy values drop out, so callers can write
// cn("kp-button", isActive && "kp-button-active") without a ternary.
export function cn(...parts: Array<string | false | null | undefined>): string {
    return parts.filter(Boolean).join(" ");
}
