// Team → Devices (E2E-09): owners and managers revoke a member's install
// (signed out, refused in the workspace) and restore it; only owners touch an
// owner's; a server without the device registry stays read-only; the demo
// changes nothing on any server.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ToastViewport } from "@/components/toast";
import { DevicesPanel, type DeviceViewer } from "./devices-panel";
import { resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const OWNER = "owner-1";
const MEMBER = "member-1";

function json(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return { ok: status < 300, status, statusText: status < 300 ? "OK" : "Error", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
const calls = (path: string, method = "GET") =>
  fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").split("?")[0] === path && (i?.method ?? "GET") === method);

const names: Record<string, string> = { [OWNER]: "Olivia Owner", [MEMBER]: "Sam Member" };
const nameOf = (id: string, hint: string | null) => names[id] ?? hint ?? "Team member";

function viewer(patch: Partial<DeviceViewer> = {}): DeviceViewer {
  return { id: "manager-1", isOwner: false, canManageDevices: true, isOwnerId: (id) => id === OWNER, ...patch };
}

function backend(opts: { registry?: unknown; registryStatus?: number } = {}) {
  let revoked = false;
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "").split("?")[0];
    const method = init?.method ?? "GET";
    if (url === "/api/app/presence/team") {
      return json({
        items: [
          { userId: MEMBER, userName: "sam", deviceId: "d-sam", deviceName: "SAM-PC", platform: "windows", status: "tracking", lastSeenAt: new Date().toISOString(), online: true },
          { userId: OWNER, userName: "olivia", deviceId: "d-own", deviceName: "OWN-MAC", platform: "macos", status: "idle", lastSeenAt: new Date().toISOString(), online: true },
        ],
      });
    }
    if (url === "/api/app/reporting/devices") return json([]);
    if (url === "/api/app/device" && method === "GET") {
      if (opts.registryStatus) return json({}, opts.registryStatus);
      return json(opts.registry ?? { items: [{ userId: MEMBER, deviceId: "d-sam", isRevoked: revoked, reason: revoked ? "Laptop lost" : null }] });
    }
    if (url === "/api/app/device/revoke-member" && method === "POST") {
      revoked = true;
      return json({ id: "r1" });
    }
    if (url === "/api/app/device/restore" && method === "POST") {
      revoked = false;
      return json({ deviceId: "d-sam", isRevoked: false });
    }
    throw new Error(`unexpected ${method} ${url}`);
  });
}

const renderPanel = (v: DeviceViewer | undefined, live = true) =>
  render(
    <>
      <DevicesPanel live={live} zone={{ timeZone: "UTC", ready: true }} nameOf={nameOf} viewer={v} />
      <ToastViewport />
    </>,
  );

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  localStorage.setItem("dosi-token", "jwt");
  localStorage.setItem("dosi-workspace", WS);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("revoking a member's device (live)", () => {
  it("revokes after the explanation, then shows it revoked with its reason and offers Restore", async () => {
    backend();
    const user = userEvent.setup();
    renderPanel(viewer());
    const table = await screen.findByRole("table");
    await user.click(within(table).getByRole("button", { name: "Revoke SAM-PC (Sam Member)" }));
    const dialog = screen.getByRole("dialog", { name: "Revoke SAM-PC?" });
    expect(dialog).toHaveTextContent(/sam member's desktop agent on it is signed out right away/i);
    await user.type(within(dialog).getByLabelText(/reason/i), "Laptop lost");
    await user.click(within(dialog).getByRole("button", { name: "Revoke device" }));

    await waitFor(() => expect(calls("/api/app/device/revoke-member", "POST")).toHaveLength(1));
    expect(JSON.parse(String(calls("/api/app/device/revoke-member", "POST")[0][1]?.body))).toEqual({ userId: MEMBER, deviceId: "d-sam", reason: "Laptop lost" });
    const restore = await within(table).findByRole("button", { name: "Restore SAM-PC (Sam Member)" });
    expect(within(table).getByText("Revoked · signed out")).toBeInTheDocument();
    expect(within(table).getByText("Reason: Laptop lost")).toBeInTheDocument();

    await user.click(restore);
    await waitFor(() => expect(calls("/api/app/device/restore", "POST")).toHaveLength(1));
    expect(await within(table).findByRole("button", { name: "Revoke SAM-PC (Sam Member)" })).toBeInTheDocument();
  });

  it("leaves an owner's device to owners", async () => {
    backend();
    const { unmount } = renderPanel(viewer());
    const table = await screen.findByRole("table");
    await within(table).findByRole("button", { name: "Revoke SAM-PC (Sam Member)" });
    expect(within(table).queryByRole("button", { name: /revoke own-mac/i })).not.toBeInTheDocument();
    unmount();
    renderPanel(viewer({ isOwner: true, id: "owner-2" }));
    expect(await within(await screen.findByRole("table")).findByRole("button", { name: "Revoke OWN-MAC (Olivia Owner)" })).toBeInTheDocument();
  });

  it("stays read-only on a server without the device registry, or without the grant", async () => {
    backend({ registryStatus: 404 });
    const { unmount } = renderPanel(viewer());
    await screen.findByRole("table");
    await waitFor(() => expect(calls("/api/app/device")).toHaveLength(1));
    expect(screen.queryByRole("button", { name: /^revoke/i })).not.toBeInTheDocument();
    unmount();
    backend();
    renderPanel(viewer({ canManageDevices: false }));
    await screen.findByRole("table");
    await waitFor(() => expect(calls("/api/app/device").length).toBeGreaterThan(1));
    expect(screen.queryByRole("button", { name: /^revoke/i })).not.toBeInTheDocument();
  });
});

describe("revoking in the demo", () => {
  it("changes the list for this visit only and sends nothing", async () => {
    localStorage.removeItem("dosi-token");
    const user = userEvent.setup();
    renderPanel(viewer({ isOwner: true, isOwnerId: () => false }), false);
    const [first] = await screen.findAllByRole("button", { name: /^revoke /i });
    await user.click(first);
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Revoke device" }));
    expect(await screen.findAllByText(/demo workspace — this lasts until you reload/i)).not.toHaveLength(0);
    expect(screen.getAllByRole("button", { name: /^restore /i }).length).toBeGreaterThan(0);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
