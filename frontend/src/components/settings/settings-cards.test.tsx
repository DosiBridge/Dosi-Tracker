// The Settings journeys (live): your devices (revoke), deleting your account
// (blocked while the sole owner, confirmed with the current password), the
// owner's retention choices (asks before deleting more) and deleting the
// workspace. Only global fetch is stubbed.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { ToastViewport } from "@/components/toast";
import { MyDevicesCard } from "./my-devices-card";
import { DeleteAccountCard } from "./delete-account-card";
import { RetentionCard } from "./retention-card";
import { WorkspaceDangerCard } from "./workspace-danger-card";
import { resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const ME = "11111111-1111-1111-1111-111111111111";

function json(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
type Route = (url: string, method: string, body: unknown) => Response | undefined;
function backend(route: Route) {
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    const answer = route(url, method, init?.body ? JSON.parse(String(init.body)) : undefined);
    if (!answer) throw new Error(`unexpected ${method} ${url}`);
    return answer;
  });
}
const calls = (path: string, method = "GET") =>
  fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "") === path && (i?.method ?? "GET") === method);
const bodyOf = (path: string, method: string, n = 0) => JSON.parse(String(calls(path, method)[n][1]?.body));

const withToasts = (ui: React.ReactElement) =>
  render(
    <>
      {ui}
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

describe("Your devices", () => {
  const devices = [
    { userId: ME, deviceId: "d1", deviceName: "MY-PC", platform: "windows", osVersion: "Windows 11", agentVersion: "0.3.0", lastSeenAt: new Date().toISOString(), isRevoked: false },
    { userId: ME, deviceId: "d2", deviceName: "OLD-MAC", platform: "macos", isRevoked: true, reason: "Sold" },
    { userId: "someone-else", deviceId: "d3", deviceName: "THEIR-PC", isRevoked: false },
  ];

  it("lists only my installs, and revokes one after saying what happens", async () => {
    backend((url, method) => {
      if (url === "/api/app/device" && method === "GET") return json({ items: devices });
      if (url === "/api/app/device/revoke" && method === "POST") return json({ id: "r1" });
    });
    const user = userEvent.setup();
    withToasts(<MyDevicesCard live userId={ME} manager={false} />);
    const list = await screen.findByRole("list", { name: "Your devices" });
    expect(within(list).getByText("MY-PC")).toBeInTheDocument();
    expect(within(list).getByText("Revoked")).toBeInTheDocument();
    expect(within(list).queryByText("THEIR-PC")).not.toBeInTheDocument();

    await user.click(within(list).getByRole("button", { name: "Revoke MY-PC" }));
    const dialog = screen.getByRole("dialog", { name: "Revoke MY-PC?" });
    expect(dialog).toHaveTextContent(/signed out right away/i);
    expect(dialog).toHaveTextContent(/only a workspace owner or manager can restore it/i);
    await user.type(within(dialog).getByLabelText(/reason/i), "Stolen");
    await user.click(within(dialog).getByRole("button", { name: "Revoke device" }));
    await waitFor(() => expect(calls("/api/app/device/revoke", "POST")).toHaveLength(1));
    expect(bodyOf("/api/app/device/revoke", "POST")).toEqual({ deviceId: "d1", reason: "Stolen" });
    expect(await screen.findByText("MY-PC revoked")).toBeInTheDocument();
    await waitFor(() => expect(calls("/api/app/device")).toHaveLength(2));
  });

  it("keeps the dialog open with the reason when the server refuses", async () => {
    backend((url, method) => {
      if (url === "/api/app/device" && method === "GET") return json({ items: devices });
      if (url === "/api/app/device/revoke") return json({ error: { message: "Nope." } }, 400);
    });
    const user = userEvent.setup();
    render(<MyDevicesCard live userId={ME} manager={false} />);
    await user.click(await screen.findByRole("button", { name: "Revoke MY-PC" }));
    const dialog = screen.getByRole("dialog", { name: "Revoke MY-PC?" });
    await user.click(within(dialog).getByRole("button", { name: "Revoke device" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Nope.");
  });

  it("says when there are none, when the server can't list them, and in the demo", async () => {
    backend((url) => (url === "/api/app/device" ? json({ items: [] }) : undefined));
    const { unmount } = render(<MyDevicesCard live userId={ME} manager />);
    expect(await screen.findByText(/none yet/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /team → devices/i })).toHaveAttribute("href", "/team");
    unmount();
    backend((url) => (url === "/api/app/device" ? json({}, 404) : undefined));
    const second = render(<MyDevicesCard live userId={ME} manager={false} />);
    expect(await screen.findByText(/can't list devices yet/i)).toBeInTheDocument();
    second.unmount();
    render(<MyDevicesCard live={false} userId={ME} manager={false} />);
    expect(screen.getByText(/the demo workspace has no devices/i)).toBeInTheDocument();
  });

  it("a11y: your devices card has no violations", async () => {
    backend((url) => (url === "/api/app/device" ? json({ items: devices }) : undefined));
    const { container } = render(<MyDevicesCard live userId={ME} manager={false} />);
    await screen.findByRole("list", { name: "Your devices" });
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("Delete your account", () => {
  it("is blocked while I'm a workspace's only owner, naming it", async () => {
    backend((url) =>
      url === "/api/app/my-account/deletion-check" ? json({ canDelete: false, isPlatformAdministrator: false, soleOwnerOf: [{ id: WS, name: "Acme" }] }) : undefined,
    );
    render(<DeleteAccountCard live onDeleted={vi.fn()} />);
    const list = await screen.findByRole("list", { name: /workspaces you are the only owner of/i });
    expect(within(list).getByText("Acme")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /delete my account/i })).not.toBeInTheDocument();
  });

  it("is never offered to a platform administrator", async () => {
    backend((url) => (url === "/api/app/my-account/deletion-check" ? json({ canDelete: false, isPlatformAdministrator: true, soleOwnerOf: [] }) : undefined));
    render(<DeleteAccountCard live onDeleted={vi.fn()} />);
    expect(await screen.findByText(/platform administrator's account can't be deleted/i)).toBeInTheDocument();
  });

  it("deletes after the current password, and signs out", async () => {
    const onDeleted = vi.fn();
    let first = true;
    backend((url, method) => {
      if (url === "/api/app/my-account/deletion-check") return json({ canDelete: true, isPlatformAdministrator: false, soleOwnerOf: [] });
      if (url === "/api/app/my-account/delete" && method === "POST") {
        if (first) {
          first = false;
          return json({ error: { code: "Tracker:InvalidPassword", message: "wrong" } }, 403);
        }
        return json(undefined, 204);
      }
    });
    const user = userEvent.setup();
    render(<DeleteAccountCard live onDeleted={onDeleted} />);
    await user.click(await screen.findByRole("button", { name: /delete my account…/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete your account?" });
    const confirm = within(dialog).getByRole("button", { name: /^delete my account$/i });
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByLabelText(/current password/i), "wrong-one");
    await user.click(confirm);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("That isn't your current password.");
    expect(onDeleted).not.toHaveBeenCalled();
    await user.click(confirm);
    await waitFor(() => expect(onDeleted).toHaveBeenCalledTimes(1));
    expect(bodyOf("/api/app/my-account/delete", "POST", 1)).toEqual({ password: "wrong-one" });
    const [, init] = calls("/api/app/my-account/delete", "POST")[0];
    expect((init?.headers as Record<string, string>)["X-Workspace-Id"]).toBeUndefined();
  });

  it("offers a retry when the check fails, and nothing in the demo", async () => {
    backend((url) => (url === "/api/app/my-account/deletion-check" ? json({ error: { message: "x" } }, 500) : undefined));
    const { unmount } = render(<DeleteAccountCard live onDeleted={vi.fn()} />);
    expect(await screen.findByRole("button", { name: /try again/i })).toBeInTheDocument();
    unmount();
    render(<DeleteAccountCard live={false} onDeleted={vi.fn()} />);
    expect(screen.getByText(/demo workspace has no account of yours/i)).toBeInTheDocument();
  });
});

describe("Retention (owners)", () => {
  function retentionBackend(opts: { capture?: unknown; activity?: unknown; settings?: Record<string, unknown> } = {}) {
    const settings = { defaultTimeZone: "Asia/Dhaka", newProjectDefaults: { intervalMinutes: 10 }, memberCaptureDeleteMinutes: 60, ...opts.settings };
    backend((url, method, body) => {
      if (url === `/api/app/workspaces/${WS}/capture-retention`) {
        return method === "PUT" ? json(body) : json(opts.capture ?? { isEnabled: false, retentionDays: 30, isEnforced: true });
      }
      if (url === `/api/app/workspaces/${WS}/activity-retention`) {
        return method === "PUT" ? json(body) : json(opts.activity ?? { isEnabled: false, retentionDays: 365, isEnforced: true });
      }
      if (url === "/api/app/workspace/settings") return method === "PUT" ? json(body) : json(settings);
    });
  }

  it("turns capture retention on after a confirmation, and sends only the policy", async () => {
    retentionBackend();
    const user = userEvent.setup();
    withToasts(<RetentionCard live demoNote="demo" />);
    const section = (await screen.findByRole("heading", { name: "Screenshots and webcam frames" })).closest("section")!;
    await user.click(within(section).getByRole("switch"));
    const days = within(section).getByLabelText(/delete captures older than/i);
    await user.clear(days);
    await user.type(days, "14");
    await user.click(screen.getByRole("button", { name: "Save retention" }));
    const confirm = screen.getByRole("dialog", { name: "Delete older data automatically?" });
    expect(calls(`/api/app/workspaces/${WS}/capture-retention`, "PUT")).toHaveLength(0);
    await user.click(within(confirm).getByRole("button", { name: /save and delete older data/i }));
    await waitFor(() => expect(calls(`/api/app/workspaces/${WS}/capture-retention`, "PUT")).toHaveLength(1));
    expect(bodyOf(`/api/app/workspaces/${WS}/capture-retention`, "PUT")).toEqual({ isEnabled: true, retentionDays: 14 });
    expect(bodyOf(`/api/app/workspaces/${WS}/activity-retention`, "PUT")).toEqual({ isEnabled: false, retentionDays: 365 });
    // The members' window didn't change: workspace settings aren't rewritten.
    expect(calls("/api/app/workspace/settings", "PUT")).toHaveLength(0);
    expect(await screen.findByText("Retention saved")).toBeInTheDocument();
  });

  it("refuses days out of range before sending, and saves a longer window without asking", async () => {
    retentionBackend({ activity: { isEnabled: true, retentionDays: 365, isEnforced: true } });
    const user = userEvent.setup();
    render(<RetentionCard live demoNote="demo" />);
    const section = (await screen.findByRole("heading", { name: "Tracked time" })).closest("section")!;
    const days = within(section).getByLabelText(/delete time blocks older than/i);
    await user.clear(days);
    await user.type(days, "30");
    expect(within(section).getByText(/from 90 to 3650/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save retention" })).toBeDisabled();
    await user.clear(days);
    await user.type(days, "400");
    await user.click(screen.getByRole("button", { name: "Save retention" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() => expect(calls(`/api/app/workspaces/${WS}/activity-retention`, "PUT")).toHaveLength(1));
    expect(bodyOf(`/api/app/workspaces/${WS}/activity-retention`, "PUT")).toEqual({ isEnabled: true, retentionDays: 400 });
  });

  it("changes the members' own-capture window without touching the other defaults", async () => {
    retentionBackend();
    const user = userEvent.setup();
    render(<RetentionCard live demoNote="demo" />);
    const window = await screen.findByLabelText(/members may delete their own captures/i);
    expect(screen.getByText(/^1 hour\./)).toBeInTheDocument();
    await user.clear(window);
    await user.type(window, "0");
    expect(screen.getByText(/^Not allowed\./)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save retention" }));
    await waitFor(() => expect(calls("/api/app/workspace/settings", "PUT")).toHaveLength(1));
    expect(bodyOf("/api/app/workspace/settings", "PUT")).toEqual({
      defaultTimeZone: "Asia/Dhaka",
      newProjectDefaults: { intervalMinutes: 10 },
      memberCaptureDeleteMinutes: 0,
    });
  });

  it("says when the operator turned the clean-up off", async () => {
    retentionBackend({ capture: { isEnabled: true, retentionDays: 30, isEnforced: false } });
    render(<RetentionCard live demoNote="demo" />);
    expect(await screen.findByText(/operator has turned the automatic clean-up off/i)).toBeInTheDocument();
  });

  it("hides what an older server lacks", async () => {
    backend((url) => {
      if (url.endsWith("/capture-retention")) return json({ isEnabled: false, retentionDays: 30 });
      if (url.endsWith("/activity-retention")) return json({}, 404);
      if (url === "/api/app/workspace/settings") return json({ defaultTimeZone: null, newProjectDefaults: {} });
    });
    render(<RetentionCard live demoNote="demo" />);
    expect(await screen.findByRole("heading", { name: "Screenshots and webcam frames" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Tracked time" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/members may delete their own captures/i)).not.toBeInTheDocument();
  });

  it("a11y: retention card has no violations", async () => {
    retentionBackend({ capture: { isEnabled: true, retentionDays: 30, isEnforced: false } });
    const { container } = render(<RetentionCard live demoNote="demo" />);
    await screen.findByRole("heading", { name: "Tracked time" });
    expect(await axe(container)).toHaveNoViolations();
  });

  it("changes nothing in the demo", async () => {
    const user = userEvent.setup();
    withToasts(<RetentionCard live={false} demoNote="Demo workspace — nothing was saved." />);
    await user.click(screen.getByRole("button", { name: "Save retention" }));
    expect(await screen.findByText("Demo workspace — nothing was saved.")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("Delete this workspace (owners)", () => {
  it("deletes after typing the name, then forgets the workspace and goes to the chooser", async () => {
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/settings", assign });
    backend((url, method) => (url === `/api/app/workspaces/${WS}/delete` && method === "POST" ? json({ id: WS, deletionTime: "x", purgeAfter: "2026-11-02T00:00:00Z" }) : undefined));
    const user = userEvent.setup();
    render(<WorkspaceDangerCard live workspaceName="Acme" demoNote="demo" />);
    await user.click(screen.getByRole("button", { name: /delete workspace…/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete Acme?" });
    await user.type(within(dialog).getByLabelText(/type acme to confirm/i), "Acme");
    await user.click(within(dialog).getByRole("button", { name: "Delete workspace" }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/workspaces"));
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("sends nothing in the demo", async () => {
    const user = userEvent.setup();
    withToasts(<WorkspaceDangerCard live={false} workspaceName="Acme Studio" demoNote="Demo note." />);
    await user.click(screen.getByRole("button", { name: /delete workspace…/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete Acme Studio?" });
    await user.type(within(dialog).getByLabelText(/type acme studio to confirm/i), "acme studio");
    await user.click(within(dialog).getByRole("button", { name: "Delete workspace" }));
    expect(await screen.findByText("Demo note.")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
