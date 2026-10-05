// ActivitiesPage in demo mode (no backend token): with no live session the
// page must fall back to the seeded tenant activities — the same demo-data
// idiom as /projects, /team and /dashboard — instead of rendering only the
// "No activities match your filters" empty state. Role scoping still applies:
// owners see the whole team, workers only their own rows.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ActivitiesPage from "@/app/(dashboard)/activities/page";
import { activities as seededActivities, projectById, userById, users } from "@/lib/tenant-data";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
  // Never-settling fetch keeps the test deterministic and forces the demo path
  // (demo mode must not call it at all — the page's useApi is disabled offline).
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

  it("opens the demo drawer with a mock preview and the apps table — no timeline, no network", async () => {
    const user = userEvent.setup();
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });

    await user.click(screen.getAllByRole("button", { name: /^Open session:/ })[0]);
    const dialog = screen.getByRole("dialog", { name: "Activity detail" });

    expect(within(dialog).getByText("Mock preview")).toBeInTheDocument();
    const apps = within(dialog).getByRole("table", { name: /per app and window/ });
    expect(within(apps).getAllByRole("row").length).toBeGreaterThan(1);
    // Demo rows carry no per-minute timeline, so the section is simply absent.
    expect(within(dialog).queryByRole("heading", { name: "Activity timeline" })).not.toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "Running programs" })).toBeInTheDocument();
    expect(fetch).not.toHaveBeenCalled();
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

describe("activities page — demo presence, notes and days", () => {
  it("'Live sessions only' follows the demo roster's presence — each tracking member's latest block, never a random flag", async () => {
    const user = userEvent.setup();
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });

    await user.click(screen.getByRole("button", { name: "Filters" }));
    const tracking = users.filter((u) => u.status === "active");
    expect(screen.getByText(`${tracking.length} members are tracking now`)).toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: /Live sessions only/ }));

    const cards = screen.queryAllByRole("button", { name: /^Open session:/ });
    expect(cards.length).toBeGreaterThan(0);
    expect(cards.length).toBeLessThanOrEqual(tracking.length);
    const names = cards.map((c) => c.getAttribute("aria-label")!.replace(/^Open session: /, "").split(",")[0]);
    // Every live card belongs to a tracking member, and each member has at most one.
    for (const name of names) expect(tracking.map((u) => u.name)).toContain(name);
    expect(new Set(names).size).toBe(names.length);
    for (const card of cards) expect(card).toHaveAccessibleName(expect.stringMatching(/live now$/));
  }, RENDER_BUDGET_MS);

  it("keeps a note edit for the visit without touching the network", async () => {
    const user = userEvent.setup();
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });

    await user.click(screen.getAllByRole("button", { name: /^Open session:/ })[0]);
    const dialog = screen.getByRole("dialog", { name: "Activity detail" });
    await user.click(within(dialog).getByRole("button", { name: "Edit note" }));
    const field = within(dialog).getByRole("textbox", { name: "Task note" });
    await user.clear(field);
    await user.type(field, "Pairing on the onboarding flow{Enter}");

    expect(await within(dialog).findByText("Pairing on the onboarding flow")).toBeInTheDocument();
    expect(fetch).not.toHaveBeenCalled();
    await user.keyboard("{Escape}");
    expect(screen.getAllByText("Pairing on the onboarding flow").length).toBeGreaterThan(0);
  }, RENDER_BUDGET_MS);

  it("heads each day with its sessions, tracked and idle time, and average activity", () => {
    renderAsRole(<ActivitiesPage />, "owner", { route: "/activities" });
    const headers = screen.getAllByRole("heading", { level: 2 }).map((h) => h.nextElementSibling?.textContent ?? "");
    expect(headers.some((t) => /\d+ sessions? · .+ tracked · .+ idle · \d+% avg activity/.test(t))).toBe(true);
  }, RENDER_BUDGET_MS);
});
