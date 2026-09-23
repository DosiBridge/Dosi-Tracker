// ActivitiesPage in demo mode (no backend token): with no live session the
// page must fall back to the seeded tenant activities — the same demo-data
// idiom as /projects, /team and /dashboard — instead of rendering only the
// "No activities match your filters" empty state. Role scoping still applies:
// owners see the whole team, workers only their own rows.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ActivitiesPage from "@/app/(dashboard)/activities/page";
import { activities as seededActivities, projectById, userById } from "@/lib/tenant-data";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
  // Never-settling fetch keeps the test deterministic and forces the demo path
  // (the page's useApi always fires one request on mount).
  vi.stubGlobal("fetch", vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {})));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// The page paints one card per matching row and does not paginate, so an owner
// on the default 7-day range mounts ~160 cards. That is fast in a browser but
// slow in jsdom, and slower again under v8 coverage instrumentation — enough to
// overrun vitest's 5s default. The budget below is a HARNESS allowance, not a
// relaxed assertion: every expectation still runs and still fails on regression.
const RENDER_BUDGET_MS = 20_000;

describe("activities page — demo-mode fallback", () => {
  it("lists the seeded team activity for an owner instead of the empty state", () => {
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });

    expect(screen.queryByText("No activities match your filters")).not.toBeInTheDocument();

    // The most recent seeded activity sorts first under the default filters;
    // its card carries the seeded description, member name and project label.
    const latest = seededActivities[0];
    expect(latest).toBeDefined();
    expect(screen.getAllByText(latest.description).length).toBeGreaterThan(0);
    // The member/project filter selects also list each name/title once as an
    // <option> — more than one match proves at least one activity CARD too.
    expect(screen.getAllByText(userById(latest.userId)!.name).length).toBeGreaterThan(1);
    expect(screen.getAllByText(projectById(latest.projectId)!.title).length).toBeGreaterThan(1);

    // The sort control is a controlled <select>: it can only render a label if
    // the DEFAULT sort is one of the options it offers. Anything else leaves the
    // user staring at a blank sort box on first paint.
    expect(screen.getByDisplayValue("Most recent")).toBeInTheDocument();

    // The page keeps its demo honesty notice.
    expect(screen.getByText("previews are mock placeholders")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("pages the session list instead of painting every card, and reveals the rest on demand", async () => {
    // A busy team produces hundreds of sessions; rendering them all at once
    // made this page the slowest in the app and gave the user an endless wall.
    const user = userEvent.setup();
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });

    const countShown = () => screen.getAllByRole("button", { name: /^Open session:/ }).length;
    const firstPage = countShown();
    expect(firstPage).toBeLessThanOrEqual(24);

    // The user is told what they are seeing, and nothing is silently dropped.
    const status = screen.getByText(/Showing \d+ of \d+ sessions/);
    expect(status).toBeInTheDocument();
    const total = Number(status.textContent!.match(/of (\d+)/)![1]);
    expect(total).toBeGreaterThan(firstPage);

    await user.click(screen.getByRole("button", { name: /show \d+ more/i }));
    expect(countShown()).toBeGreaterThan(firstPage);
  }, RENDER_BUDGET_MS);

  it("scopes the demo fallback to a worker's own rows", () => {
    renderAsRole(<ActivitiesPage />, "worker", { route: "/activities" }); // u2 · Tanvir Hasan

    expect(screen.queryByText("No activities match your filters")).not.toBeInTheDocument();
    // Workers get no member filter, so any member name on screen comes from cards.
    expect(screen.queryByText("All members")).not.toBeInTheDocument();
    expect(screen.getAllByText("Tanvir Hasan").length).toBeGreaterThan(0);
    for (const other of ["Ayesha Rahman", "Nusrat Jahan", "Rafiq Islam", "Sadia Akter", "Imran Kabir", "David Chen"]) {
      expect(screen.queryByText(other)).not.toBeInTheDocument();
    }
  }, RENDER_BUDGET_MS);
});
