// SettingsPage behavior: the profile form in demo mode (no token, no network)
// and in live mode (token present — loads GET /api/account/my-profile, saves
// via PUT), the personal time zone (my-preferences), the self-service
// password change, the owner's workspace defaults, and the sections that are
// honest about not existing yet. Only global fetch is stubbed.
//
// NOTE: success feedback for the profile is BOTH the inline "Saved" indicator
// next to the save button and a success toast ("Profile saved"); ToastViewport
// is mounted (withToasts) so the toast is visible to queries.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
// The browser's full IANA list (~430 zones) makes every axe scan of a page with
// a zone picker take ~10 s in jsdom; a short list keeps these page tests fast.
vi.mock("@/lib/time-zone", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/time-zone")>();
  return { ...actual, supportedTimeZones: () => ["UTC", "America/New_York", "Asia/Dhaka", "Asia/Kolkata", "Europe/Madrid"] };
});

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import SettingsPage from "@/app/(dashboard)/settings/page";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";

function jsonResponse(body: unknown, init: { status?: number; statusText?: string } = {}): Response {
  const { status = 200, statusText = "OK" } = init;
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    headers: new Headers(),
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

interface BackendOptions {
  /** Status for PUT my-profile (200 = success). */
  putStatus?: number;
  /** ABP error body for a failing PUT. */
  putError?: unknown;
  /** The stored personal time zone. */
  timeZone?: string | null;
  /** Workspace settings, or a status to fail with. */
  workspace?: unknown | { failWith: number };
  /** Status (and ABP error body) for the password change. */
  changeStatus?: number;
  changeError?: unknown;
  /** Identity roles of the signed-in user ("admin" = workspace owner). */
  roles?: string[];
}

/** Live-mode fetch dispatcher: the settings endpoints + the SessionProvider hydration calls. */
function stubLiveBackend(opts: BackendOptions = {}): void {
  const { putStatus = 200 } = opts;
  let stamp = 1;
  const profile = {
    userName: "ayesha",
    name: "Ayesha",
    surname: "Rahman",
    email: "ayesha@dosi.dev",
    phoneNumber: "01700000000",
    concurrencyStamp: `stamp-${stamp}`,
  };
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    if (url === `${API_BASE}/api/account/my-profile` && method === "GET") {
      return jsonResponse({ ...profile, concurrencyStamp: `stamp-${stamp}` });
    }
    if (url === `${API_BASE}/api/account/my-profile` && method === "PUT") {
      if (putStatus !== 200) {
        return jsonResponse(opts.putError ?? {}, { status: putStatus, statusText: putStatus === 403 ? "Forbidden" : "Internal Server Error" });
      }
      // Like ABP: every save issues a new concurrency stamp, which the NEXT save must carry.
      stamp += 1;
      return jsonResponse({ ...profile, ...JSON.parse(String(init?.body)), concurrencyStamp: `stamp-${stamp}` });
    }
    if (url === `${API_BASE}/api/account/my-profile/change-password` && method === "POST") {
      const status = opts.changeStatus ?? 204;
      if (status >= 400) return jsonResponse(opts.changeError ?? {}, { status, statusText: "Bad Request" });
      return { ok: true, status: 204, statusText: "No Content", headers: new Headers(), json: async () => null, text: async () => "" } as Response;
    }
    if (url.startsWith(`${API_BASE}/api/app/workspace/my-preferences`)) {
      if (method === "PUT") return jsonResponse(JSON.parse(String(init?.body)));
      return jsonResponse({ timeZone: opts.timeZone ?? null });
    }
    if (url.startsWith(`${API_BASE}/api/app/workspace/settings`)) {
      const ws = opts.workspace;
      if (ws && typeof ws === "object" && "failWith" in ws) {
        return jsonResponse({}, { status: (ws as { failWith: number }).failWith, statusText: "Not Found" });
      }
      if (method === "PUT") return jsonResponse(JSON.parse(String(init?.body)));
      return jsonResponse(
        ws ?? {
          defaultTimeZone: "Asia/Dhaka",
          newProjectDefaults: {
            intervalMinutes: 15,
            allowScreenshot: true,
            allowWebcam: false,
            allowKeyboard: true,
            allowMouse: true,
            allowActiveWindow: true,
            allowRunningPrograms: false,
            blurScreenshots: true,
            idleThresholdMinutes: 10,
          },
        },
      );
    }
    // SessionProvider builds the live session from these when a token exists.
    if (url.includes("/api/abp/application-configuration")) {
      return jsonResponse({
        currentUser: {
          isAuthenticated: true,
          id: "live-owner",
          tenantId: "t-1",
          userName: "ayesha",
          name: "Ayesha",
          surName: "Rahman",
          email: "ayesha@dosi.dev",
          roles: opts.roles ?? ["admin"],
        },
        currentTenant: { id: "t-1", name: "Dosi Real", isAvailable: true },
        auth: { grantedPolicies: {} },
      });
    }
    if (url.includes("/api/app/workspace/current-subscription")) return jsonResponse({});
    if (url.includes("/api/app/")) return jsonResponse({ items: [], totalCount: 0 });
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

function callsTo(path: string, method: string) {
  return fetchMock.mock.calls.filter(([u, init]) => String(u).startsWith(`${API_BASE}${path}`) && (init?.method ?? "GET") === method);
}

async function openTab(user: ReturnType<typeof userEvent.setup>, name: RegExp) {
  await user.click(screen.getByRole("tab", { name }));
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

  it("says plainly that nothing is saved in the demo workspace — never a false 'Saved'", async () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    await user.click(saveButton());

    expect(await screen.findByText(/demo workspace — nothing was saved/i)).toBeInTheDocument();
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(saveButton()).toBeEnabled();
  });

  it("offers no dead controls: no avatar upload, and notifications are marked coming soon", async () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner");

    expect(screen.queryByRole("button", { name: /change avatar/i })).not.toBeInTheDocument();
    await openTab(user, /notifications/i);
    expect(screen.getByText("Coming soon")).toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
  });

  it("explains that the demo has no account to change instead of offering a form that does nothing", async () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");

    await openTab(user, /security/i);
    expect(screen.getByText(/demo workspace has no accounts to change/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/current password/i)).not.toBeInTheDocument();
  });

  it("shows the workspace tab to owners only", () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const owner = renderAsRole(<SettingsPage />, "owner");
    expect(screen.getByRole("tab", { name: /workspace/i })).toBeInTheDocument();
    owner.unmount();

    renderAsRole(<SettingsPage />, "admin");
    expect(screen.queryByRole("tab", { name: /workspace/i })).not.toBeInTheDocument();
  });

  it("describes what the agent really collects, including idle time and blurred screenshots", async () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");

    await openTab(user, /privacy/i);
    expect(screen.getByText("Idle time")).toBeInTheDocument();
    expect(screen.getByText(/keep the away time as work/i)).toBeInTheDocument();
    expect(screen.getByText(/blurs the image before it leaves your computer/i)).toBeInTheDocument();
    expect(screen.getByText(/never what you type/i)).toBeInTheDocument();
  });

  it("moves between sections with the arrow keys (tabs)", async () => {
    fetchMock.mockRejectedValue(new Error("no backend in demo mode"));
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");

    const profile = screen.getByRole("tab", { name: /profile/i });
    expect(profile).toHaveAttribute("aria-selected", "true");
    profile.focus();
    await user.keyboard("{ArrowDown}");
    const security = screen.getByRole("tab", { name: /security/i });
    expect(security).toHaveAttribute("aria-selected", "true");
    expect(security).toHaveFocus();
    expect(screen.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", security.id);
  });
});

describe("settings page — live profile", () => {
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
    expect(await screen.findByText("Profile saved")).toBeInTheDocument(); // success toast

    const putCall = callsTo("/api/account/my-profile", "PUT")[0];
    expect(putCall).toBeDefined();
    expect(String(putCall?.[0])).toBe(`${API_BASE}/api/account/my-profile`);
    const body = JSON.parse(String(putCall?.[1]?.body)) as Record<string, unknown>;
    expect(body.surname).toBe("Rahman-Khan");
    // Extra backend fields (concurrency stamp) must survive the round-trip.
    expect(body.concurrencyStamp).toBe("stamp-1");
  });

  it("a second save carries the concurrency stamp the first save returned (no stale-stamp failure)", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    const surname = await screen.findByDisplayValue("Rahman");
    await user.clear(surname);
    await user.type(surname, "Khan");
    await user.click(saveButton());
    await waitFor(() => expect(callsTo("/api/account/my-profile", "PUT")).toHaveLength(1));
    await waitFor(() => expect(saveButton()).toBeEnabled());

    const phone = screen.getByLabelText(/phone number/i);
    await user.clear(phone);
    await user.type(phone, "01800000000");
    await user.click(saveButton());

    await waitFor(() => expect(callsTo("/api/account/my-profile", "PUT")).toHaveLength(2));
    const second = JSON.parse(String(callsTo("/api/account/my-profile", "PUT")[1][1]?.body)) as Record<string, unknown>;
    expect(second.concurrencyStamp).toBe("stamp-2");
    expect(second.surname).toBe("Khan"); // the first save's change is kept
    expect(second.phoneNumber).toBe("01800000000");
  });

  it("a failed save shows the server's own message inline and keeps the form usable — no false Saved", async () => {
    stubLiveBackend({ putStatus: 403, putError: { error: { code: "Volo.Abp.Identity:DuplicateEmail", message: "Email 'ayesha@dosi.dev' is already taken." } } });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    await screen.findByDisplayValue("Rahman");
    await user.click(saveButton());

    expect(await screen.findByText("Email 'ayesha@dosi.dev' is already taken.")).toBeInTheDocument();
    expect(screen.queryByText(/API PUT Error/)).not.toBeInTheDocument();
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
    await waitFor(() => expect(saveButton()).toBeEnabled()); // not stuck in "Saving…"
    expect(screen.getByDisplayValue("Rahman")).toBeInTheDocument(); // form still there
  });

  it("a failure with no message falls back to a sentence and the HTTP status", async () => {
    stubLiveBackend({ putStatus: 500 });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });

    await screen.findByDisplayValue("Rahman");
    await user.click(saveButton());

    expect(await screen.findByText("Couldn't save your profile. (HTTP 500)")).toBeInTheDocument();
  });
});

describe("settings page — live time zone", () => {
  it("loads the saved zone and saves a new one to my-preferences", async () => {
    stubLiveBackend({ timeZone: "Europe/Madrid" });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker", { withToasts: true });

    const select = (await screen.findByLabelText(/^time zone$/i)) as HTMLSelectElement;
    await waitFor(() => expect(select).toHaveValue("Europe/Madrid"));
    await user.selectOptions(select, "Asia/Dhaka");
    await user.click(screen.getByRole("button", { name: /save time zone/i }));

    await waitFor(() => expect(callsTo("/api/app/workspace/my-preferences", "PUT")).toHaveLength(1));
    expect(JSON.parse(String(callsTo("/api/app/workspace/my-preferences", "PUT")[0][1]?.body))).toEqual({ timeZone: "Asia/Dhaka" });
    expect(await screen.findByText("Time zone saved")).toBeInTheDocument();
  });

  it("clearing the choice sends null (follow the workspace default, then the browser)", async () => {
    stubLiveBackend({ timeZone: "Europe/Madrid" });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker", { withToasts: true });

    const select = (await screen.findByLabelText(/^time zone$/i)) as HTMLSelectElement;
    await waitFor(() => expect(select).toHaveValue("Europe/Madrid"));
    await user.selectOptions(select, "");
    await user.click(screen.getByRole("button", { name: /save time zone/i }));

    await waitFor(() => expect(callsTo("/api/app/workspace/my-preferences", "PUT")).toHaveLength(1));
    expect(JSON.parse(String(callsTo("/api/app/workspace/my-preferences", "PUT")[0][1]?.body))).toEqual({ timeZone: null });
  });
});

describe("settings page — live password change", () => {
  async function fillAndSubmit(user: ReturnType<typeof userEvent.setup>, current: string, next: string, confirm: string) {
    if (current) await user.type(screen.getByLabelText(/^current password$/i), current);
    if (next) await user.type(screen.getByLabelText(/^new password$/i), next);
    if (confirm) await user.type(screen.getByLabelText(/confirm new password/i), confirm);
    await user.click(screen.getByRole("button", { name: /change password/i }));
  }

  it("posts the current and new password to ABP's change-password endpoint and confirms", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");
    await openTab(user, /security/i);

    await fillAndSubmit(user, "Dt1!one-time", "brand-new-9", "brand-new-9");

    await waitFor(() => expect(callsTo("/api/account/my-profile/change-password", "POST")).toHaveLength(1));
    const body = JSON.parse(String(callsTo("/api/account/my-profile/change-password", "POST")[0][1]?.body));
    expect(body).toEqual({ currentPassword: "Dt1!one-time", newPassword: "brand-new-9" });
    expect(await screen.findByText(/your password was changed/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^current password$/i)).toHaveValue(""); // cleared after success
  });

  it("checks the two new entries match before sending anything", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");
    await openTab(user, /security/i);

    await fillAndSubmit(user, "old-secret-1", "brand-new-9", "brand-new-8");

    expect(await screen.findByText(/the two new passwords don't match/i)).toBeInTheDocument();
    expect(callsTo("/api/account/my-profile/change-password", "POST")).toHaveLength(0);
  });

  it("shows the server's validation message when it refuses", async () => {
    stubLiveBackend({
      changeStatus: 400,
      changeError: { error: { message: "Your request is not valid!", validationErrors: [{ message: "Incorrect password.", members: ["currentPassword"] }] } },
    });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");
    await openTab(user, /security/i);

    await fillAndSubmit(user, "wrong-one-1", "brand-new-9", "brand-new-9");

    expect(await screen.findByText("Incorrect password.")).toBeInTheDocument();
  });

  it("uses the browser's password-manager hints on each field", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "worker");
    await openTab(user, /security/i);

    expect(screen.getByLabelText(/^current password$/i)).toHaveAttribute("autocomplete", "current-password");
    expect(screen.getByLabelText(/^new password$/i)).toHaveAttribute("autocomplete", "new-password");
    expect(screen.getByLabelText(/confirm new password/i)).toHaveAttribute("autocomplete", "new-password");
  });
});

describe("settings page — live workspace defaults (owner)", () => {
  it("loads the defaults, and saving PUTs the edited settings", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner", { withToasts: true });
    await screen.findByDisplayValue("Rahman");
    await openTab(user, /workspace/i);

    const zone = (await screen.findByLabelText(/default time zone/i)) as HTMLSelectElement;
    await waitFor(() => expect(zone).toHaveValue("Asia/Dhaka"));
    // Loaded values are shown: the 15-minute block is selected and idle detection reads 10 min.
    expect(screen.getByRole("radio", { name: "15 min" })).toBeChecked();
    expect(screen.getByLabelText(/idle detection/i)).toHaveValue("10");

    await user.click(screen.getByRole("radio", { name: "30 min" }));
    await user.selectOptions(screen.getByLabelText(/idle detection/i), "0");
    await user.click(screen.getByRole("switch", { name: /webcam/i }));
    await user.click(screen.getByRole("button", { name: /save workspace defaults/i }));

    await waitFor(() => expect(callsTo("/api/app/workspace/settings", "PUT")).toHaveLength(1));
    const body = JSON.parse(String(callsTo("/api/app/workspace/settings", "PUT")[0][1]?.body));
    expect(body).toEqual({
      defaultTimeZone: "Asia/Dhaka",
      newProjectDefaults: {
        intervalMinutes: 30,
        allowScreenshot: true,
        allowWebcam: true,
        allowKeyboard: true,
        allowMouse: true,
        allowActiveWindow: true,
        allowRunningPrograms: false,
        blurScreenshots: true,
        idleThresholdMinutes: 0,
      },
    });
    expect(await screen.findByText("Workspace defaults saved")).toBeInTheDocument();
  });

  it("an older server without workspace settings says so instead of showing controls that save nowhere", async () => {
    stubLiveBackend({ workspace: { failWith: 404 } });
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    renderAsRole(<SettingsPage />, "owner");
    await screen.findByDisplayValue("Rahman");
    await openTab(user, /workspace/i);

    expect(await screen.findByText(/doesn't support workspace defaults yet/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /save workspace defaults/i })).not.toBeInTheDocument();
  });
});

describe("settings page — accessibility", () => {
  it("every demo-mode profile field is reachable by its visible label", () => {
    renderAsRole(<SettingsPage />, "owner");

    // Queried by accessible name: fails if the label/input association breaks.
    expect(screen.getByLabelText(/full name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^email$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/designation/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/^time zone$/i)).toBeInTheDocument();
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

  it("a11y: the live security form has no axe violations", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    const { container } = renderAsRole(<SettingsPage />, "worker");
    await openTab(user, /security/i);
    await screen.findByLabelText(/^current password$/i);
    expect(await axe(container)).toHaveNoViolations();
  });

  it("a11y: the workspace defaults form has no axe violations", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "live-token");
    const user = userEvent.setup();
    const { container } = renderAsRole(<SettingsPage />, "owner");
    await screen.findByDisplayValue("Rahman");
    await openTab(user, /workspace/i);
    await screen.findByLabelText(/default time zone/i);
    const panel = within(container).getByRole("tabpanel");
    expect(await axe(panel)).toHaveNoViolations();
  });
});
