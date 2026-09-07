// ActivitiesPage in demo mode (no backend token): with no live session the
// page must fall back to the seeded tenant activities — the same demo-data
// idiom as /projects, /team and /dashboard — instead of rendering only the
// "No activities match your filters" empty state. Role scoping still applies:
// owners see the whole team, workers only their own rows.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen } from "@testing-library/react";
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

    // The page keeps its demo honesty notice.
    expect(screen.getByText("previews are mock placeholders")).toBeInTheDocument();
  });

  it("scopes the demo fallback to a worker's own rows", () => {
    renderAsRole(<ActivitiesPage />, "worker", { route: "/activities" }); // u2 · Tanvir Hasan

    expect(screen.queryByText("No activities match your filters")).not.toBeInTheDocument();
    // Workers get no member filter, so any member name on screen comes from cards.
    expect(screen.queryByText("All members")).not.toBeInTheDocument();
    expect(screen.getAllByText("Tanvir Hasan").length).toBeGreaterThan(0);
    for (const other of ["Ayesha Rahman", "Nusrat Jahan", "Rafiq Islam", "Sadia Akter", "Imran Kabir", "David Chen"]) {
      expect(screen.queryByText(other)).not.toBeInTheDocument();
    }
  });
});
