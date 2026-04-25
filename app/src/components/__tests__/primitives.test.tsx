import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Button, Card, Chip, EntityChip, IconButton, TooltipProvider } from "../index";
import { Heart } from "lucide-react";

/*
 * Phase 1 primitive contract tests. These don't try to be exhaustive —
 * they pin down the keyboard-and-aria behaviours that subsequent phases
 * rely on. If a test breaks because the look-and-feel changed, fix the
 * test; if it breaks because Tab/Enter/Escape stopped working, fix the
 * primitive.
 */

describe("Button", () => {
    it("invokes onClick on Enter/Space when focused", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(<Button onClick={onClick}>Save</Button>);
        const button = screen.getByRole("button", { name: "Save" });
        button.focus();
        await user.keyboard("{Enter}");
        expect(onClick).toHaveBeenCalledTimes(1);
        await user.keyboard(" ");
        expect(onClick).toHaveBeenCalledTimes(2);
    });

    it("blocks clicks when loading", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(
            <Button onClick={onClick} loading>
                Saving
            </Button>,
        );
        await user.click(screen.getByRole("button"));
        expect(onClick).not.toHaveBeenCalled();
    });

    it("renders the configured variant via data-attribute", () => {
        render(<Button variant="danger">Delete</Button>);
        expect(screen.getByRole("button")).toHaveAttribute("data-variant", "danger");
    });
});

describe("IconButton", () => {
    it("requires a label that becomes both aria-label and tooltip", () => {
        render(
            <TooltipProvider>
                <IconButton icon={<Heart />} label="Favorite" />
            </TooltipProvider>,
        );
        expect(screen.getByRole("button")).toHaveAttribute("aria-label", "Favorite");
    });
});

describe("Chip", () => {
    it("renders as a button when onClick is provided", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(<Chip onClick={onClick}>RAW</Chip>);
        await user.click(screen.getByRole("button", { name: "RAW" }));
        expect(onClick).toHaveBeenCalled();
    });

    it("renders as a static span when no onClick is provided", () => {
        render(<Chip>Tokyo</Chip>);
        // No role="button" — just a span.
        expect(screen.queryByRole("button")).toBeNull();
        expect(screen.getByText("Tokyo")).toBeTruthy();
    });

    it("reflects active state via data-attribute", () => {
        render(
            <Chip active onClick={() => undefined}>
                RAW
            </Chip>,
        );
        expect(screen.getByRole("button")).toHaveAttribute("data-active", "true");
    });
});

describe("Card", () => {
    it("activates onClick via Enter and Space", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(
            <Card onClick={onClick}>
                <span>Italy 2024</span>
            </Card>,
        );
        const card = screen.getByRole("button");
        card.focus();
        await user.keyboard("{Enter}");
        expect(onClick).toHaveBeenCalledTimes(1);
        await user.keyboard(" ");
        expect(onClick).toHaveBeenCalledTimes(2);
    });

    it("does not become a button when no onClick is provided", () => {
        render(
            <Card>
                <span>Static</span>
            </Card>,
        );
        expect(screen.queryByRole("button")).toBeNull();
    });
});

describe("EntityChip", () => {
    it("dispatches onClick with the entity payload", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(
            <EntityChip
                entity={{ kind: "place", placeId: "JP:tokyo", name: "Tokyo, Japan" }}
                onClick={onClick}
            />,
        );
        await user.click(screen.getByRole("button", { name: /Tokyo, Japan/ }));
        expect(onClick).toHaveBeenCalledTimes(1);
        expect(onClick.mock.calls[0][0]).toMatchObject({
            kind: "place",
            placeId: "JP:tokyo",
            name: "Tokyo, Japan",
        });
    });

    it("renders disabled when no onClick handler is provided", () => {
        render(
            <EntityChip
                entity={{ kind: "category", key: "raw", label: "RAW" }}
            />,
        );
        expect(screen.getByRole("button", { name: /RAW/ })).toBeDisabled();
    });

    it("falls back to truncated node id when peer label is missing", () => {
        render(
            <EntityChip
                entity={{ kind: "peer", nodeIdHex: "abcd1234567890efghij" }}
                onClick={() => undefined}
            />,
        );
        // Truncated form: "abcd…ghij"
        expect(screen.getByRole("button").textContent).toMatch(/abcd…ghij/);
    });
});
