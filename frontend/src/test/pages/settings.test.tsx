// SettingsPage behavior: the profile form in demo mode (no token, no network)
// and in live mode (token present — loads GET /api/account/my-profile, saves
// via PUT). Only global fetch is stubbed.
//
// NOTE: success feedback on this page is the inline "Saved" indicator next to
// the save button — the page does not use the toast() system at all (verified:
// no toast import). ToastViewport is still mounted (withToasts) so if a toast
// ever WERE fired these queries would see it; assertions target the real
// inline feedback.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import SettingsPage from "@/app/(dashboard)/settings/page";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:44342";

function jsonResponse(body: unknown, init: { status?: number; statusText?: string } = {}): Response {
  const { status = 200, statusText = "OK" } = init;
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const saveButton = () => screen.getByRole("button", { name: /save changes/i });

/** Live-mode fetch dispatcher: profile GET/PUT + the SessionProvider hydration calls. */
function stubLiveBackend(opts: { putStatus?: number } = {}): void {
  const { putStatus = 200 } = opts;
  const profile = {
    userName: "ayesha",
    name: "Ayesha",
    surname: "Rahman",
    email: "ayesha@dosi.dev",
    phoneNumber: "01700000000",
    concurrencyStamp: "stamp-1",
  };
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    if (url === `${API_BASE}/api/account/my-profile` && method === "GET") {
      return jsonResponse(profile);
    }
    if (url === `${API_BASE}/api/account/my-profile` && method === "PUT") {
      if (putStatus !== 200) {
        return jsonResponse({}, { status: putStatus, statusText: "Internal Server Error" });
      }
      return jsonResponse({ ...profile, ...JSON.parse(String(init?.body)) });
    }
    // SessionProvider hydration fires these when a token exists.
    if (url.includes("/api/app/project") || url.includes("/api/app/activity")) {
      return jsonResponse([]);
    }
    if (url.includes("/api/abp/application-configuration")) {
      return jsonResponse({});
    }
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

describe("settings page — demo mode (no backend session)", () => {
  it("renders the profile form pre-filled with the signed-in demo user", () => {
    // Any fetch here would be a bug — demo mode must be fully offline.
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    expect(screen.getByDisplayValue("Ayesha Rahman")).toBeInTheDocument();
    expect(screen.getByDisplayValue("ayesha@dosi.dev")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Engineering Lead")).toBeInTheDocument();
    expect(screen.getByText("Owner")).toBeInTheDocument(); // role badge
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("tells the user preferences are device-local rather than claiming they were saved", async () => {
    // Previously this showed a green "Saved" for preference tabs that never
    // persist anywhere — feedback asserting a round-trip that never happened.
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    await user.click(saveButton());

    expect(await screen.findByText(/applied on this device/i)).toBeInTheDocument();
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(saveButton()).toBeEnabled();
  });
});

describe("settings page — live mode (backend token present)", () => {
  it("loads the profile from the backend, and saving PUTs the edits back with success feedback", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token"); // before render — the page reads it on mount
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    const surname = await screen.findByDisplayValue("Rahman");
    await user.clear(surname);
    await user.type(surname, "Rahman-Khan");
    await user.click(saveButton());

    expect(await screen.findByText("Saved")).toBeInTheDocument();

    const putCall = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(putCall).toBeDefined();
    expect(String(putCall?.[0])).toBe(`${API_BASE}/api/account/my-profile`);
    const body = JSON.parse(String(putCall?.[1]?.body)) as Record<string, unknown>;
    expect(body.surname).toBe("Rahman-Khan");
    // Extra backend fields (concurrency stamp) must survive the round-trip.
    expect(body.concurrencyStamp).toBe("stamp-1");
  });

  it("a failed save surfaces an inline error and keeps the form usable — no crash, no false Saved", async () => {
    stubLiveBackend({ putStatus: 500 });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    await screen.findByDisplayValue("Rahman");
    await user.click(saveButton());

    expect(await screen.findByText(/API PUT Error/)).toBeInTheDocument();
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
    await waitFor(() => expect(saveButton()).toBeEnabled()); // not stuck in "Saving…"
    expect(screen.getByDisplayValue("Rahman")).toBeInTheDocument(); // form still there
  });
});

describe("settings page — accessibility of the profile form", () => {
  it("every demo-mode profile field is reachable by its visible label", () => {
    renderAsRole(<SettingsPage />, "owner");

    // Queried by accessible name: fails if the label/input association breaks.
    expect(screen.getByLabelText(/full name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^email$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/designation/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/timezone/i)).toBeInTheDocument();
  });

  it("every live-mode profile field is reachable by its visible label", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    renderAsRole(<SettingsPage />, "owner");

    await screen.findByDisplayValue("Rahman");
    expect(screen.getByLabelText(/first name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/surname/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^email$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/username/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/phone number/i)).toBeInTheDocument();
  });

  it("a11y: the profile tab has no axe violations", async () => {
    const { container } = renderAsRole(<SettingsPage />, "owner");
    expect(await axe(container)).toHaveNoViolations();
  });
});
