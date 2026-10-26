// The sign-up plan picker (FE-12, E2E-08): the server's own plans, cheapest
// first, sent by id — and no picker at all while the deployment doesn't bill.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SignupPlanPicker, planOptionLabel } from "./signup-plan-picker";
import { resetBillingModeForTests, useBillingMode } from "@/hooks/use-billing-mode";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: status < 300 ? "OK" : "Error", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
const PLANS = [
  { id: "plan-b", name: "Business", pricePerUser: 12, maxSeats: 50, trialDays: 14 },
  { id: "plan-f", name: "Free", pricePerUser: 0, maxSeats: 3, trialDays: 0 },
];

function backend(mode: unknown, modeStatus = 200) {
  fetchMock.mockImplementation(async (input) => {
    const url = String(input).replace(API, "");
    if (url === "/api/app/workspace/billing-mode") return json(mode, modeStatus);
    if (url === "/api/app/workspace/available-plans") return json({ items: PLANS });
    throw new Error(`unexpected ${url}`);
  });
}

beforeEach(() => {
  vi.stubEnv("DOSI_DEMO_MODE", "0");
  resetBillingModeForTests();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("SignupPlanPicker", () => {
  it("offers the server's plans cheapest first and reports the chosen id", async () => {
    backend({ enabled: true });
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<SignupPlanPicker onChange={onChange} />);
    const select = await screen.findByLabelText("Choose a plan");
    expect([...select.querySelectorAll("option")].map((o) => o.textContent)).toEqual([
      "Free — $0 · up to 3 seats",
      "Business — $12/user/mo · 14-day trial",
    ]);
    expect(onChange).toHaveBeenLastCalledWith("plan-f");
    await user.selectOptions(select, "plan-b");
    expect(onChange).toHaveBeenLastCalledWith("plan-b");
  });

  it("shows nothing, and asks for no plans, while the deployment doesn't bill", async () => {
    backend({ enabled: false });
    const onChange = vi.fn();
    render(<SignupPlanPicker onChange={onChange} />);
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(null));
    expect(screen.queryByLabelText("Choose a plan")).not.toBeInTheDocument();
    expect(fetchMock.mock.calls.some(([u]) => String(u).includes("available-plans"))).toBe(false);
  });

  it("keeps the plans on an older server that can't say whether it bills", async () => {
    backend({}, 404);
    render(<SignupPlanPicker onChange={vi.fn()} />);
    expect(await screen.findByLabelText("Choose a plan")).toBeInTheDocument();
  });

  it("shows nothing when the plans can't be read (the server then starts on its free plan)", async () => {
    fetchMock.mockImplementation(async (input) =>
      String(input).includes("billing-mode") ? json({ enabled: true }) : json({ error: { message: "down" } }, 500),
    );
    const onChange = vi.fn();
    render(<SignupPlanPicker onChange={onChange} />);
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).includes("available-plans"))).toBe(true));
    expect(screen.queryByLabelText("Choose a plan")).not.toBeInTheDocument();
  });

  it("labels a plan from what the server says, nothing invented", () => {
    expect(planOptionLabel({ name: "Pro", pricePerUser: 8, maxSeats: 0, trialDays: 0 })).toBe("Pro — $8/user/mo");
    expect(planOptionLabel({ name: null, pricePerUser: null, maxSeats: null, trialDays: null })).toBe("Plan — $0");
  });
});

describe("useBillingMode", () => {
  function Probe() {
    const mode = useBillingMode();
    return <span data-testid="mode">{mode.ready ? String(mode.enabled) : "asking"}</span>;
  }

  it("asks the server once per page view, shared by every caller", async () => {
    backend({ enabled: false });
    render(
      <>
        <Probe />
        <Probe />
      </>,
    );
    await waitFor(() => expect(screen.getAllByTestId("mode").map((e) => e.textContent)).toEqual(["false", "false"]));
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("asks again after a failed request", async () => {
    fetchMock.mockRejectedValueOnce(new TypeError("offline"));
    const first = render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("mode")).toHaveTextContent("null"));
    first.unmount();
    backend({ enabled: true });
    render(<Probe />);
    await waitFor(() => expect(screen.getByTestId("mode")).toHaveTextContent("true"));
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("bills in the demo without asking", () => {
    vi.stubEnv("DOSI_DEMO_MODE", "1");
    render(<Probe />);
    expect(screen.getByTestId("mode")).toHaveTextContent("true");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
