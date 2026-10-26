// Billing journeys (E2E-08), live through the real providers: a deployment
// that doesn't bill shows no plans, trials, seats or invoices anywhere; one
// that does lets the owner move to a free plan the team fits, or cancel a
// paid subscription, each after a confirmation. Only global fetch is stubbed.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import BillingPage from "@/app/(dashboard)/billing/page";
import { Topbar } from "@/components/layout/topbar";
import { Sidebar } from "@/components/layout/sidebar";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";

function json(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return { ok: status < 300, status, statusText: status < 300 ? "OK" : "Error", json: async () => JSON.parse(text), text: async () => text } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
const calls = (path: string, method = "GET") =>
  fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").split("?")[0] === path && (i?.method ?? "GET") === method);

interface Options {
  billing?: boolean;
  freeSeats?: number;
  subscription?: { planId: string; status: string };
  changeError?: unknown;
}

function backend(opts: Options = {}) {
  let subscription = opts.subscription ?? { planId: "plan-s", status: "active" };
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: OWNER, tenantId: "t-1", userName: "mira", name: "Mira", surName: "Okafor", email: "mira@realco.test", roles: ["admin"] },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } },
      });
    }
    if (url === "/api/app/workspace/billing-mode") return json({ enabled: opts.billing ?? true });
    if (url.startsWith("/api/app/team/members")) {
      return json({
        items: [
          { userId: OWNER, userName: "mira", name: "Mira", isOwner: true, isManager: true, projectIds: [PROJECT] },
          { userId: WORKER, userName: "sam", name: "Sam", isOwner: false, isManager: false, projectIds: [PROJECT] },
        ],
      });
    }
    if (url.startsWith("/api/app/project")) return json({ items: [{ id: PROJECT, title: "Apollo", isArchived: false }] });
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json(subscription);
    if (url.startsWith("/api/app/workspace/available-plans")) {
      return json({
        items: [
          { id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25, trialDays: 14 },
          { id: "plan-f", name: "Free", pricePerUser: 0, maxSeats: opts.freeSeats ?? 3, trialDays: 0 },
          { id: "plan-b", name: "Business", pricePerUser: 12, maxSeats: 100 },
        ],
      });
    }
    if (url.startsWith("/api/app/billing/invoices")) return json({ items: [] });
    if (url === "/api/app/billing/change-plan" && method === "POST") {
      if (opts.changeError) return json(opts.changeError, 409);
      subscription = { planId: JSON.parse(String(init?.body)).planId, status: "active" };
      return json({ id: "s1", ...subscription });
    }
    if (url === "/api/app/billing/cancel" && method === "POST") {
      subscription = { ...subscription, status: "canceled" };
      return json({ id: "s1", ...subscription });
    }
    if (url.startsWith("/api/app/notification/my-notifications")) return json({ items: [] });
    if (url.startsWith("/api/app/reporting/")) return json({ totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: null, perUser: [], perProject: [] });
    if (url.startsWith("/api/app/presence/team")) return json({ items: [] });
    if (url.startsWith("/api/app/workspace/")) return json({});
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  localStorage.setItem("dosi-token", "jwt-live");
});

afterEach(() => vi.unstubAllGlobals());

describe("a deployment that doesn't bill", () => {
  it("shows no plans, trials or invoices on the billing page", async () => {
    backend({ billing: false, subscription: { planId: "plan-s", status: "trialing" } });
    renderWithProviders(<BillingPage />);
    expect(await screen.findByRole("heading", { name: /billing is turned off on this server/i })).toBeInTheDocument();
    expect(screen.queryByText(/available plans/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/trial ends/i)).not.toBeInTheDocument();
    expect(calls("/api/app/billing/invoices")).toHaveLength(0);
  });

  it("keeps the plan chip and the Billing link out of the shell", async () => {
    backend({ billing: false });
    renderWithProviders(
      <>
        <Sidebar collapsed={false} onToggle={() => {}} mobileOpen={false} onMobileClose={() => {}} />
        <Topbar onMenuClick={() => {}} />
      </>,
    );
    await waitFor(() => expect(screen.getByRole("link", { name: /settings/i })).toBeInTheDocument());
    await waitFor(() => expect(calls("/api/app/workspace/billing-mode")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("link", { name: /billing & plan/i })).not.toBeInTheDocument());
    expect(screen.queryByTitle("Billing & plan")).not.toBeInTheDocument();
    expect(screen.queryByText("Starter")).not.toBeInTheDocument();
  });
});

describe("an owner's self-service plan changes (billing on)", () => {
  it("moves to a free plan the team fits, after confirming", async () => {
    backend();
    const user = userEvent.setup();
    renderWithProviders(<BillingPage />, { withToasts: true });
    await user.click(await screen.findByRole("button", { name: "Switch to Free" }));
    const dialog = screen.getByRole("dialog", { name: "Switch to Free?" });
    expect(calls("/api/app/billing/change-plan", "POST")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Switch to Free" }));
    await waitFor(() => expect(calls("/api/app/billing/change-plan", "POST")).toHaveLength(1));
    expect(JSON.parse(String(calls("/api/app/billing/change-plan", "POST")[0][1]?.body))).toEqual({ planId: "plan-f" });
    expect(await screen.findByText("Moved to Free")).toBeInTheDocument();
    // The session is rebuilt from the new subscription: Free is now the current plan.
    await waitFor(() => expect(screen.getAllByRole("button", { name: "Current plan" })).toHaveLength(1));
    expect(screen.queryByRole("button", { name: "Switch to Free" })).not.toBeInTheDocument();
  });

  it("explains a free plan the team has outgrown instead of offering it", async () => {
    backend({ freeSeats: 1 });
    renderWithProviders(<BillingPage />);
    expect(await screen.findByText(/your team uses 2 seats; free allows 1/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Switch to Free" })).toBeDisabled();
  });

  it("asks support for a paid plan (the platform assigns those)", async () => {
    backend();
    renderWithProviders(<BillingPage />);
    expect(await screen.findByRole("link", { name: /request upgrade/i })).toHaveAttribute("href", expect.stringMatching(/^mailto:/));
  });

  it("cancels a paid subscription after saying what it means", async () => {
    backend();
    const user = userEvent.setup();
    renderWithProviders(<BillingPage />, { withToasts: true });
    await user.click(await screen.findByRole("button", { name: "Cancel subscription" }));
    const dialog = screen.getByRole("dialog", { name: "Cancel the Starter subscription?" });
    expect(dialog).toHaveTextContent(/time tracking pauses for everyone until you switch to a free plan/i);
    await user.click(within(dialog).getByRole("button", { name: "Cancel subscription" }));
    await waitFor(() => expect(calls("/api/app/billing/cancel", "POST")).toHaveLength(1));
    expect(await screen.findByText("Subscription canceled")).toBeInTheDocument();
    // Canceled: the page now says what that means, and offers the free plan to keep tracking.
    expect(await screen.findByText(/subscription is canceled/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel subscription" })).not.toBeInTheDocument();
  });

  it("offers the free plan on a trial, and shows a refusal's reason", async () => {
    backend({ subscription: { planId: "plan-s", status: "trialing" }, changeError: { error: { code: "Tracker:SeatLimitReached", message: "seats" } } });
    const user = userEvent.setup();
    renderWithProviders(<BillingPage />, { withToasts: true });
    expect(await screen.findByText(/to keep tracking after the trial, switch to free/i)).toBeInTheDocument();
    const [first] = screen.getAllByRole("button", { name: "Switch to Free" });
    await user.click(first);
    await user.click(within(screen.getByRole("dialog", { name: "Switch to Free?" })).getByRole("button", { name: "Switch to Free" }));
    expect(await screen.findByText(/more members than that plan allows/i)).toBeInTheDocument();
  });
});
