import { beforeEach, describe, expect, it } from "vitest";
import { useSession } from "../session";
import { entityToView, navigateToEntity } from "../navigation";

/*
 * Phase 2 back-stack + entity-dispatch invariants. Phase 4 expands the
 * entityToView coverage when place/peer surfaces are wired in; for now
 * we pin down:
 *
 *   - setView resets the stack to a single frame
 *   - pushView appends; popView removes; popView on a single-frame
 *     stack is a no-op
 *   - view always equals backstack[backstack.length - 1]
 *   - entityToView routes every kind somewhere reasonable
 */

beforeEach(() => {
    useSession.getState().reset();
});

describe("session back-stack", () => {
    it("starts with a single for-you frame", () => {
        const { view, backstack } = useSession.getState();
        expect(backstack).toHaveLength(1);
        expect(backstack[0]).toEqual({ kind: "for-you" });
        expect(view).toEqual({ kind: "for-you" });
    });

    it("setView replaces the entire stack with one frame", () => {
        useSession.getState().setView({ kind: "albums" });
        useSession.getState().pushView({ kind: "person", id: 1, name: "Mom" });
        expect(useSession.getState().backstack).toHaveLength(2);
        useSession.getState().setView({ kind: "search" });
        const { view, backstack } = useSession.getState();
        expect(backstack).toEqual([{ kind: "search" }]);
        expect(view).toEqual({ kind: "search" });
    });

    it("pushView appends; popView removes", () => {
        const s = useSession.getState();
        s.setView({ kind: "albums" });
        s.pushView({ kind: "album", id: 1, name: "Italy 2024" });
        s.pushView({ kind: "asset", id: 42, back: { kind: "album", id: 1, name: "Italy 2024" } });
        const { view, backstack } = useSession.getState();
        expect(backstack).toHaveLength(3);
        expect(view.kind).toBe("asset");
        useSession.getState().popView();
        expect(useSession.getState().view.kind).toBe("album");
        useSession.getState().popView();
        expect(useSession.getState().view.kind).toBe("albums");
    });

    it("popView is a no-op when only one frame remains", () => {
        useSession.getState().setView({ kind: "library" });
        const before = useSession.getState().backstack;
        useSession.getState().popView();
        expect(useSession.getState().backstack).toBe(before);
    });

    it("view always equals backstack[backstack.length - 1]", () => {
        const s = useSession.getState();
        s.setView({ kind: "albums" });
        s.pushView({ kind: "search" });
        s.pushView({ kind: "people" });
        s.popView();
        const { view, backstack } = useSession.getState();
        expect(view).toEqual(backstack[backstack.length - 1]);
    });

    it("reset wipes the stack back to for-you default", () => {
        useSession.getState().setView({ kind: "albums" });
        useSession.getState().pushView({ kind: "person", id: 1, name: "Mom" });
        useSession.getState().setHiddenUnlocked(true);
        useSession.getState().reset();
        const s = useSession.getState();
        expect(s.backstack).toEqual([{ kind: "for-you" }]);
        expect(s.hiddenUnlocked).toBe(false);
        expect(s.session).toBe(null);
    });
});

describe("entityToView dispatcher", () => {
    it("routes person → person view", () => {
        expect(
            entityToView({ kind: "person", id: 1, name: "Mom" }),
        ).toEqual({ kind: "person", id: 1, name: "Mom" });
    });

    it("routes place → place view", () => {
        expect(
            entityToView({
                kind: "place",
                placeId: "JP:tokyo",
                name: "Tokyo, Japan",
            }),
        ).toEqual({
            kind: "place",
            placeId: "JP:tokyo",
            name: "Tokyo, Japan",
        });
    });

    it("routes trip → album with source=trip", () => {
        expect(
            entityToView({ kind: "trip", id: 7, name: "Italy 2024" }),
        ).toEqual({ kind: "album", id: 7, name: "Italy 2024", source: "trip" });
    });

    it("routes album → album", () => {
        expect(
            entityToView({ kind: "album", id: 1, name: "Family" }),
        ).toEqual({ kind: "album", id: 1, name: "Family" });
    });

    it("routes peer → settings/peers", () => {
        expect(
            entityToView({
                kind: "peer",
                nodeIdHex: "abc123",
            }),
        ).toEqual({ kind: "settings", section: "peers" });
    });

    it("routes filter-only entities (camera/lens/category/date) to search", () => {
        for (const e of [
            { kind: "camera" as const, make: "SONY" },
            { kind: "lens" as const, lens: "FE 24-70" },
            {
                kind: "category" as const,
                key: "raw" as const,
                label: "RAW",
            },
            {
                kind: "date" as const,
                utcDay: 19500,
                label: "March 2024",
            },
        ]) {
            expect(entityToView(e)?.kind).toBe("search");
        }
    });
});

describe("navigateToEntity helper", () => {
    it("delegates to pushView with the dispatched view", () => {
        const calls: { kind: string }[] = [];
        navigateToEntity(
            { kind: "person", id: 1, name: "Mom" },
            (v) => calls.push(v),
        );
        expect(calls).toEqual([{ kind: "person", id: 1, name: "Mom" }]);
    });
});
