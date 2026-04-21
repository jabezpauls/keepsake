import tseslint from "@typescript-eslint/eslint-plugin";
import tsparser from "@typescript-eslint/parser";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default [
    {
        ignores: ["dist", "src-tauri/target", "src-tauri/gen", "node_modules"],
    },
    {
        files: ["**/*.{ts,tsx}"],
        languageOptions: {
            parser: tsparser,
            parserOptions: {
                ecmaVersion: 2022,
                sourceType: "module",
                ecmaFeatures: { jsx: true },
            },
            globals: {
                window: "readonly",
                document: "readonly",
                console: "readonly",
                HTMLElement: "readonly",
                setTimeout: "readonly",
                clearTimeout: "readonly",
            },
        },
        plugins: {
            "@typescript-eslint": tseslint,
            "react-hooks": reactHooks,
            "react-refresh": reactRefresh,
        },
        rules: {
            "react-hooks/rules-of-hooks": "error",
            "react-hooks/exhaustive-deps": "warn",
            "react-refresh/only-export-components": "warn",
            "@typescript-eslint/no-unused-vars": [
                "error",
                { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
            ],
        },
    },
];
