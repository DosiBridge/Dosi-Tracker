// Acceptance spec — Feature: Host billing lifecycle.
// Gherkin documentation: src/test/acceptance/features/host-billing-lifecycle.feature
//
// Business rule under test: the platform operator can move a paying customer
// through its subscription — change its plan, activate it, record paid
// invoices — and keep the plan catalogue current; each billing change asks
// first. Live mode, against a small in-memory platform.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import HostTenantsPage from "@/app/(host)/host/tenants/page";
import HostBillingPage from "@/app/(host)/host/billing/page";
import HostPlansPage from "@/app/(host)/host/plans/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ACME = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

function calls(path: string, method: string) {
  return fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").startsWith(path) && (i?.method ?? "GET") === method);
}

const PLANS = [
  { id: "plan-free", name: "Free", pricePerUser: 0, maxSeats: 3, trialDays: 0 },
  { id: "plan-starter", name: "Starter", pricePerUser: 6, maxSeats: 25, trialDays: 14 },
  { id: "plan-business", name: "Business", pricePerUser: 12, maxSeats: 100, trialDays: 14 },
];

/**
 * The in-memory platform. `subscriptions` is what GET tenant-subscriptions
 * answers; left out, the server is an older one without that endpoint (and
 * without suspend), as the contract's own host additions assume.
 */
function platform(opts: { deletePlanStatus?: number; subscriptions?: unknown[] } = {}) {
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/app/platform/tenant-subscriptions") && opts.subscriptions) return json({ items: opts.subscriptions });
    if (url.startsWith("/api/app/platform/tenant-subscriptions")) return json({ error: { code: null, message: "Not found" } }, 404);
    if (url.startsWith("/api/app/platform/suspend-subscription") && method === "POST" && opts.subscriptions) return json(null, 204);
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: "host-1", tenantId: null, userName: "admin", roles: ["admin"] },
        currentTenant: { id: null, name: null, isAvailable: false },
        auth: { grantedPolicies: { "Tracker.Platform": true } },
      });
    }
    if (url.startsWith("/api/multi-tenancy/tenants")) return json({ items: [{ id: ACME, name: "Acme" }], totalCount: 1 });
    if (url.startsWith("/api/app/platform/plans")) return json({ items: PLANS });
    if (url.startsWith("/api/app/platform/overview")) {
      return json({
        tenantCount: 1,
        activeSubscriptions: 1,
        trialingSubscriptions: 0,
        mrr: 48,
        paidSeats: 8,
        totalInvoiced: 96,
        pendingInvoices: 1,
        plans: [{ planId: "plan-starter", name: "Starter", pricePerUser: 6, subscriberCount: 1 }],
      });
    }
    if (url.startsWith("/api/app/platform/invoices")) {
      return json({
        items: [
          { id: "inv-1", tenantId: ACME, tenantName: "Acme", amount: 48, status: "pending", dueDate: "2026-10-01T00:00:00Z", billingPeriodStart: "2026-09-01T00:00:00Z" },
          { id: "inv-2", tenantId: ACME, tenantName: "=HYPERLINK(\"https://evil.example\",\"Open\")", amount: 48, status: "paid", dueDate: "2026-09-01T00:00:00Z" },
          { id: "inv-3", tenantId: "dddddddd-dddd-dddd-dddd-dddddddddddd", tenantName: "", amount: 12, status: "paid", dueDate: "2026-08-01T00:00:00Z" },
        ],
        totalCount: 3,
      });
    }
    if (url.startsWith("/api/app/platform/change-tenant-plan") && method === "POST") return json(null, 204);
    if (url.startsWith("/api/app/platform/activate-subscription") && method === "POST") return json(null, 204);
    if (url.startsWith("/api/app/platform/mark-invoice-paid") && method === "POST") return json(null, 204);
    if (url === "/api/app/platform/plan" && method === "POST") return json({ id: "plan-new", ...JSON.parse(String(init?.body)) });
    if (/^\/api\/app\/platform\/[^/]+\/plan$/.test(url) && method === "PUT") return json({ id: url.split("/")[4], ...JSON.parse(String(init?.body)) });
    if (/^\/api\/app\/platform\/[^/]+\/plan$/.test(url) && method === "DELETE") {
      if (opts.deletePlanStatus) return json({ error: { code: "Tracker:PlanInUse", message: "Plan is in use." } }, opts.deletePlanStatus);
      return json(null, 204);
    }
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  localStorage.setItem("dosi-token", "jwt-host");
  platform();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openAcme(user: ReturnType<typeof userEvent.setup>) {
  renderWithProviders(<HostTenantsPage />, { route: "/host/tenants", withToasts: true });
  // The table renders a phone layout and a desktop one; either opens the same drawer.
  await user.click((await screen.findAllByRole("button", { name: "Manage Acme" }))[0]);
  return screen.getByRole("dialog", { name: "Tenant details" });
}

describe("Feature: Host billing lifecycle", () => {
  describe("Scenario: Change a tenant's plan", () => {
    it("Given a live tenant When the host chooses Business and confirms Then the platform moves it to that plan", async () => {
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      const select = await within(drawer).findByLabelText(/move to plan/i);
      await waitFor(() => expect(within(select).getAllByRole("option").length).toBe(4));
      await user.selectOptions(select, "plan-business");
      await user.click(within(drawer).getByRole("button", { name: /change plan/i }));

      const confirm = screen.getByRole("dialog", { name: /move acme to business\?/i });
      expect(within(confirm).getByText(/seat limit becomes 100/i)).toBeInTheDocument();
      expect(calls("/api/app/platform/change-tenant-plan", "POST")).toHaveLength(0);
      await user.click(within(confirm).getByRole("button", { name: "Change plan" }));

      await waitFor(() => expect(calls(`/api/app/platform/change-tenant-plan?tenantId=${ACME}&planId=plan-business`, "POST")).toHaveLength(1));
      expect(await screen.findByText("Acme moved to Business")).toBeInTheDocument();
    });
  });

  describe("Scenario: Activate a subscription", () => {
    it("Given a tenant's details When the host activates and confirms Then the subscription becomes active", async () => {
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      await user.click(within(drawer).getByRole("button", { name: /^activate$/i }));
      await user.click(within(screen.getByRole("dialog", { name: /activate acme's subscription\?/i })).getByRole("button", { name: "Activate subscription" }));

      await waitFor(() => expect(calls(`/api/app/platform/activate-subscription?tenantId=${ACME}`, "POST")).toHaveLength(1));
      expect(await screen.findByText("Acme's subscription is active")).toBeInTheDocument();
    });
  });

  describe("Scenario: See each tenant's plan and status", () => {
    const acme = { tenantId: ACME, tenantName: "Acme", subscriptionId: "sub-1", planId: "plan-starter", planName: "Starter", status: "active", trialEndsAt: null, seatsUsed: 8, maxSeats: 25 };

    it("Given a server that reports subscriptions Then the table and the drawer show the real plan, status and seats", async () => {
      platform({ subscriptions: [acme] });
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      const table = screen.getAllByRole("table")[0];
      expect(within(table).getByText("Starter")).toBeInTheDocument();
      expect(within(table).getByText("Active")).toBeInTheDocument();
      expect(within(drawer).getByText("Seats used").nextElementSibling).toHaveTextContent("8 of 25");
      expect(within(drawer).queryByText(/doesn't report each tenant's current plan/i)).not.toBeInTheDocument();
    });

    it("Given an older server without them Then plan and status read '—', never a guessed 'Active', and nothing offers to suspend", async () => {
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      const table = screen.getAllByRole("table")[0];
      expect(within(table).queryByText("Active")).not.toBeInTheDocument();
      expect(within(drawer).getByText(/doesn't report each tenant's current plan/i)).toBeInTheDocument();
      expect(within(drawer).queryByRole("button", { name: /suspend/i })).not.toBeInTheDocument();
      // Signing in as a tenant is a demo-only tool: a live console doesn't show it at all.
      expect(within(drawer).queryByRole("button", { name: /login as tenant/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Log in as Acme" })).not.toBeInTheDocument();
    });

    it("Given an active tenant When the host suspends it and confirms Then the platform suspends its subscription", async () => {
      platform({ subscriptions: [acme] });
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      await user.click(await within(drawer).findByRole("button", { name: /^suspend$/i }));
      const confirm = screen.getByRole("dialog", { name: /suspend acme\?/i });
      expect(within(confirm).getByText(/its 8 members can no longer record time/i)).toBeInTheDocument();
      expect(calls("/api/app/platform/suspend-subscription", "POST")).toHaveLength(0);
      await user.click(within(confirm).getByRole("button", { name: "Suspend workspace" }));

      await waitFor(() => expect(calls(`/api/app/platform/suspend-subscription?tenantId=${ACME}`, "POST")).toHaveLength(1));
      expect(await screen.findByText("Acme suspended")).toBeInTheDocument();
    });

    it("Given a suspended tenant When the host reactivates it Then its subscription is activated", async () => {
      platform({ subscriptions: [{ ...acme, status: "suspended" }] });
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      expect(within(screen.getAllByRole("table")[0]).getByText("Suspended")).toBeInTheDocument();
      await user.click(await within(drawer).findByRole("button", { name: /reactivate/i }));
      await user.click(within(screen.getByRole("dialog", { name: /activate acme's subscription\?/i })).getByRole("button", { name: "Activate subscription" }));
      await waitFor(() => expect(calls(`/api/app/platform/activate-subscription?tenantId=${ACME}`, "POST")).toHaveLength(1));
    });
  });

  describe("Scenario: Record a paid invoice", () => {
    it("Given a pending invoice When the host marks it paid on Platform Billing Then it is recorded", async () => {
      const user = userEvent.setup();
      renderWithProviders(<HostBillingPage />, { route: "/host/billing", withToasts: true });

      const buttons = await screen.findAllByRole("button", { name: /mark acme's \$48 invoice due 2026-10-01 as paid/i });
      await user.click(buttons[0]);
      const confirm = screen.getByRole("dialog", { name: /mark this invoice paid\?/i });
      expect(within(confirm).getByText(/sep 2026/i)).toBeInTheDocument();
      await user.click(within(confirm).getByRole("button", { name: "Mark paid" }));

      await waitFor(() => expect(calls("/api/app/platform/mark-invoice-paid?invoiceId=inv-1", "POST")).toHaveLength(1));
    });

    it("Given a tenant's details When the host marks one of its invoices paid Then it is recorded too", async () => {
      const user = userEvent.setup();
      const drawer = await openAcme(user);

      await user.click(await within(drawer).findByRole("button", { name: /mark the invoice due 2026-10-01 paid/i }));
      await user.click(within(screen.getByRole("dialog", { name: /mark this invoice paid\?/i })).getByRole("button", { name: "Mark paid" }));
      await waitFor(() => expect(calls("/api/app/platform/mark-invoice-paid?invoiceId=inv-1", "POST")).toHaveLength(1));
    });
  });

  describe("Scenario: Export invoices safely", () => {
    it("Given a tenant named like a formula When exporting Then the CSV carries it as text", async () => {
      const blobs: Blob[] = [];
      const createObjectURL = vi.fn((blob: Blob) => {
        blobs.push(blob);
        return "blob:invoices";
      });
      vi.stubGlobal("URL", Object.assign(Object.create(URL), { createObjectURL, revokeObjectURL: vi.fn() }));
      const user = userEvent.setup();
      renderWithProviders(<HostBillingPage />, { route: "/host/billing", withToasts: true });

      await screen.findAllByText("Acme");
      await user.click(screen.getByRole("button", { name: /export invoices/i }));

      expect(blobs).toHaveLength(1);
      const csv = await blobs[0].text();
      expect(csv).toContain("Invoice,Tenant,Billing period,Due date,Amount USD,Status");
      expect(csv).toContain(`"'=HYPERLINK(""https://evil.example"",""Open"")"`);
      expect(csv).not.toMatch(/(^|,)=HYPERLINK/m);
      expect(csv).toContain("Sep 2026");
    });
  });

  describe("Scenario: Keep the plan catalogue current", () => {
    it("Given the plans page When the host creates a plan Then it is posted with its limits", async () => {
      const user = userEvent.setup();
      renderWithProviders(<HostPlansPage />, { route: "/host/plans", withToasts: true });

      await screen.findAllByText("Business");
      await user.click(screen.getByRole("button", { name: /new plan/i }));
      const dialog = screen.getByRole("dialog", { name: "New plan" });
      await user.type(within(dialog).getByLabelText(/^name$/i), "Team");
      await user.type(within(dialog).getByLabelText(/price \/ user/i), "9.5");
      await user.type(within(dialog).getByLabelText(/seat limit/i), "50");
      await user.clear(within(dialog).getByLabelText(/trial days/i));
      await user.type(within(dialog).getByLabelText(/trial days/i), "7");
      await user.click(within(dialog).getByRole("button", { name: "Create plan" }));

      await waitFor(() => expect(calls("/api/app/platform/plan", "POST")).toHaveLength(1));
      expect(JSON.parse(String(calls("/api/app/platform/plan", "POST")[0][1]?.body))).toEqual({ name: "Team", pricePerUser: 9.5, maxSeats: 50, trialDays: 7 });
    });

    it("Given an invalid plan When submitting Then the fields say what's wrong and nothing is sent", async () => {
      const user = userEvent.setup();
      renderWithProviders(<HostPlansPage />, { route: "/host/plans" });

      await screen.findAllByText("Business");
      await user.click(screen.getByRole("button", { name: /new plan/i }));
      const dialog = screen.getByRole("dialog", { name: "New plan" });
      await user.type(within(dialog).getByLabelText(/seat limit/i), "0");
      await user.click(within(dialog).getByRole("button", { name: "Create plan" }));

      expect(within(dialog).getByText("Give the plan a name.")).toBeInTheDocument();
      expect(within(dialog).getByText(/a whole number from 1 to 100,000/i)).toBeInTheDocument();
      expect(calls("/api/app/platform/plan", "POST")).toHaveLength(0);
    });

    it("Given an existing plan When the host edits its price Then it is updated by id", async () => {
      const user = userEvent.setup();
      renderWithProviders(<HostPlansPage />, { route: "/host/plans", withToasts: true });

      await user.click((await screen.findAllByRole("button", { name: "Edit Business" }))[0]);
      const dialog = screen.getByRole("dialog", { name: "Edit Business" });
      const price = within(dialog).getByLabelText(/price \/ user/i);
      await user.clear(price);
      await user.type(price, "14");
      await user.click(within(dialog).getByRole("button", { name: "Save plan" }));

      await waitFor(() => expect(calls("/api/app/platform/plan-business/plan", "PUT")).toHaveLength(1));
      expect(JSON.parse(String(calls("/api/app/platform/plan-business/plan", "PUT")[0][1]?.body))).toEqual({
        name: "Business",
        pricePerUser: 14,
        maxSeats: 100,
        trialDays: 14,
      });
    });

    it("Given a plan without tenants When the host deletes it Then it is removed; a plan with tenants can't be", async () => {
      const user = userEvent.setup();
      renderWithProviders(<HostPlansPage />, { route: "/host/plans", withToasts: true });

      for (const button of await screen.findAllByRole("button", { name: "Delete Starter" })) expect(button).toBeDisabled(); // one tenant on it
      await user.click(screen.getAllByRole("button", { name: "Delete Business" })[0]);
      await user.click(within(screen.getByRole("dialog", { name: /delete business\?/i })).getByRole("button", { name: "Delete plan" }));
      await waitFor(() => expect(calls("/api/app/platform/plan-business/plan", "DELETE")).toHaveLength(1));
    });

    it("Given the server still finds tenants on a plan When deleting Then it says to move them first", async () => {
      platform({ deletePlanStatus: 409 });
      const user = userEvent.setup();
      renderWithProviders(<HostPlansPage />, { route: "/host/plans", withToasts: true });

      await user.click((await screen.findAllByRole("button", { name: "Delete Free" }))[0]);
      await user.click(within(screen.getByRole("dialog", { name: /delete free\?/i })).getByRole("button", { name: "Delete plan" }));
      expect(await screen.findByText(/tenants are still on this plan/i)).toBeInTheDocument();
    });
  });

  describe("Scenario: Invoices of a deleted workspace", () => {
    it("Given an invoice whose tenant no longer has a name Then it reads as a deleted workspace, never as the host's", async () => {
      renderWithProviders(<HostBillingPage />, { route: "/host/billing" });
      const table = (await screen.findAllByRole("table"))[0];
      expect(await within(table).findByText("Deleted workspace")).toBeInTheDocument();
      expect(within(table).queryByText("Host")).not.toBeInTheDocument();
    });
  });

  describe("Scenario: Deleting a tenant says what really happens", () => {
    it("Given a live tenant's details Then the danger zone says sign-ins stop and the data is not erased", async () => {
      const user = userEvent.setup();
      const drawer = await openAcme(user);
      expect(within(drawer).getByText(/stops every sign-in to it/i)).toBeInTheDocument();
      expect(within(drawer).getByText(/not erased from the database/i)).toBeInTheDocument();
      expect(within(drawer).queryByText(/removes all of its data/i)).not.toBeInTheDocument();
    });
  });
});
