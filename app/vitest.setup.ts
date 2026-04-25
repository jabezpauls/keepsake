// Extends Vitest's `expect` with DOM-aware matchers (toHaveAttribute,
// toBeDisabled, toBeInTheDocument, etc.). Imported once per test run via
// vite.config.ts → test.setupFiles.
import "@testing-library/jest-dom/vitest";
