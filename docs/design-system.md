# Keepsake design system — decision log

This document records the design-system decisions taken for the
Apple/Google-grade UI overhaul (see `plans/wise-strolling-otter.md` for
the full plan). It is a *log*, not a tutorial — each decision is dated,
short, and has a rationale so future contributors understand why a
choice was made.

Status: Phase 0 (skeleton).

## Stack

| Concern | Choice | Rationale |
|---|---|---|
| Component primitives | **Radix UI** (`@radix-ui/react-{dialog,dropdown-menu,popover,tooltip,tabs,toggle-group,scroll-area,slider,toast}`) | Owns focus management, keyboard, ARIA, portal positioning, scroll-lock. Unstyled — doesn't dictate look. ~60 KB gz. |
| Iconography | **lucide-react** | 1500 icons, single style, used by Linear/Notion/Cal.com. Tree-shakable. Replaces our Unicode chevrons + the absence of icons everywhere else. |
| Animation | **framer-motion** | `layoutId` is the only sane way to do shared-element transitions (Phase 3 hero moment). `useReducedMotion` honours OS pref. ~50 KB gz. |
| Command palette | **cmdk** | The library Linear/Vercel use. Integrates cleanly with Radix's portal model. ~10 KB gz. |
| Fonts | **Inter Variable** + **JetBrains Mono Variable** (`@fontsource-variable/{inter,jetbrains-mono}`) | Loaded locally — Tauri is offline-first and cannot fetch from rsms.me. Inter for UI; JetBrains for ticket base32 / EXIF JSON / node IDs. |
| Styling | Plain CSS, file-per-feature | Tailwind would require touching every `.tsx`; CSS-in-JS adds runtime cost. The existing 1847-line `styles.css` splits into `tokens.css`, `primitives.css`, per-feature CSS. |
| Theme model | `:root[data-theme]` + `prefers-color-scheme` media query | Auto by default, explicit user override in Settings → Appearance. Light + dark token parity. |

## Rejected alternatives

- **`@radix-ui/themes`** (the full theme system) — too opinionated; we
  want the unstyled primitives so we control look-and-feel via tokens.
- **Tailwind CSS** — high migration cost (every `.tsx` touched), and the
  app's design language is deliberately constrained enough that a tiny
  primitives layer + tokens covers what Tailwind would give us.
- **Headless UI** — Radix has more primitives we need (toggle-group,
  scroll-area, toast) and is the de-facto choice in the React/Linear
  ecosystem.
- **react-aria** — excellent but lower-level than Radix; would force us
  to re-implement Radix-level conveniences. Reconsider if Radix proves
  insufficient.
- **motion-one** — smaller than framer-motion but lacks `layoutId`. The
  shared-element transition is non-negotiable for the redesign's
  signature, so framer-motion wins.

## Token tree

See `app/src/styles/tokens.css` for the canonical definitions. Summary:

- **Spacing**: 4 px base scale, `--space-0..12`.
- **Radius**: `--radius-{sm,md,lg,xl,full}`.
- **Shadow**: `--shadow-{sm,md,lg}` — three levels only; Apple/Linear
  barely use any.
- **Motion**: durations `instant/fast/base/slow/shared-layout`,
  easings `standard/decel/accel/spring` (the spring matches Linear's
  signature curve `cubic-bezier(0.32, 0.72, 0, 1)`).
- **Color**: semantic tokens (`--color-canvas`, `--color-surface-1..3`,
  `--color-text-{primary,secondary,tertiary,disabled,on-accent}`,
  `--color-border-{subtle,default,strong,focus}`), an
  accent ramp `--color-accent-50..900`, and four status colors
  (`--color-{success,warning,danger,info}-500`).
- **Type**: shorthand composite tokens (`--font-display`,
  `--font-title-{1,2}`, `--font-body`, `--font-caption`, `--font-mono`)
  for use with the CSS `font:` shorthand.
- **Z-layer**: `--z-{app,overlay-chrome,popover,modal,toast,cmdk}`.

## Theme model

Three states: explicit light, explicit dark, auto (default).

```html
<html data-theme="light">  <!-- forced light -->
<html data-theme="dark">   <!-- forced dark -->
<html>                     <!-- auto: follows OS via prefers-color-scheme -->
```

The token sheet uses three rule blocks:

1. `:root, :root[data-theme="light"]` — light defaults.
2. `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { ... } }` —
   automatic dark when OS prefers it AND no explicit light override.
3. `:root[data-theme="dark"]` — explicit dark, mirrors the auto block.

Ordering matters because (2) and (3) have similar specificity; (3)
appears after (2) so explicit `data-theme="dark"` always wins.

## Reduced-motion

`@media (prefers-reduced-motion: reduce)` collapses every
`--duration-*` token to `0ms`. framer-motion's `useReducedMotion()`
hook picks up the same OS pref and disables `layoutId` transitions
automatically — so respecting the pref is one media query plus one
hook call per motion site.

## Migration policy

- Phase 0 (this phase): tokens.css written, NOT yet consumed. Existing
  `styles.css` (1847 lines, 254 selectors) untouched.
- Phase 1: `primitives.css` lands; new `app/src/components/` consumes
  tokens. Old screens still on `styles.css`.
- Phase 2+: per-screen migrations onto tokens. `styles.css` shrinks.
- End of Phase 9: `styles.css` deleted; everything is tokens +
  primitives + per-feature CSS.

The legacy `--bg`, `--fg`, `--muted`, `--accent`, `--error`, `--panel`,
`--border` custom properties are **not removed** during Phase 0/1 —
removing them would break every screen at once. They survive until the
last screen migration in Phase 9.
