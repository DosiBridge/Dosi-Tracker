import { describe, expect, it } from "vitest";
import type { DeviceReportRowDto, PresenceDto } from "@/lib/api-contract";
import { buildActivity, buildUser } from "@/test/factories";
import {
  buildDeviceRows,
  compareAgentVersions,
  demoDeviceSources,
  deviceStatusText,
  deviceStatusTone,
  isComparableVersion,
  memberPresence,
  newestAgentVersion,
  osText,
} from "./devices";

const NOW = new Date("2026-10-05T12:00:00Z");
const ago = (seconds: number) => new Date(NOW.getTime() - seconds * 1000).toISOString();
const name = (userId: string, hint: string | null) => ({ u1: "Ayesha Rahman", u2: "Tanvir Hasan" })[userId] ?? hint ?? "Team member";

function presence(patch: Partial<PresenceDto> = {}): PresenceDto {
  return {
    userId: "u1",
    userName: "ayesha",
    deviceId: "d1",
    deviceName: "AYESHA-PC",
    platform: "windows",
    osVersion: "Windows 11 Pro 23H2",
    agentVersion: "0.2.0",
    status: "tracking",
    lastSeenAt: ago(30),
    online: true,
    ...patch,
  };
}

function reportRow(patch: Partial<DeviceReportRowDto> = {}): DeviceReportRowDto {
  return {
    userId: "u1",
    userName: "ayesha",
    deviceId: "d1",
    deviceName: "AYESHA-PC",
    platform: "windows",
    osVersion: "Windows 11 Pro 23H2",
    agentVersion: "0.2.0",
    lastActivityAt: ago(600),
    trackedMinutes: 120,
    ...patch,
  };
}

describe("agent versions", () => {
  it("orders versions numerically, part by part", () => {
    expect(compareAgentVersions("0.2.0", "0.10.0")).toBeLessThan(0);
    expect(compareAgentVersions("1.0.0", "0.99.9")).toBeGreaterThan(0);
    expect(compareAgentVersions("0.2", "0.2.0")).toBe(0);
    expect(compareAgentVersions("v0.3.1", "0.3.1")).toBe(0);
  });

  it("puts a pre-release before its release", () => {
    expect(compareAgentVersions("0.2.0-beta.2", "0.2.0")).toBeLessThan(0);
    expect(compareAgentVersions("0.2.0-beta.10", "0.2.0-beta.2")).toBeGreaterThan(0);
    expect(compareAgentVersions("0.2.0+build.7", "0.2.0")).toBe(0);
  });

  it("finds the newest readable version and ignores unreadable ones", () => {
    expect(newestAgentVersion(["0.1.9", "0.2.0", null, "dev", "0.2.0-rc.1"])).toBe("0.2.0");
    expect(newestAgentVersion([null, "custom"])).toBeNull();
    expect(isComparableVersion("dev")).toBe(false);
    expect(isComparableVersion(" 0.2.0 ")).toBe(true);
  });
});

describe("buildDeviceRows", () => {
  it("merges a device's heartbeat and its uploaded time into one row", () => {
    const rows = buildDeviceRows([presence()], [reportRow()], name, NOW);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      memberName: "Ayesha Rahman",
      deviceName: "AYESHA-PC",
      status: "tracking",
      online: true,
      trackedMinutes: 120,
      lastSeenAt: ago(30),
      outdated: false,
    });
  });

  it("flags only agents older than the newest version seen in the workspace", () => {
    const rows = buildDeviceRows(
      [presence(), presence({ userId: "u2", deviceId: "d2", deviceName: "TANVIR-PC", agentVersion: "0.1.9" })],
      [reportRow({ userId: "u3", deviceId: "d3", agentVersion: null, deviceName: "OLD" })],
      name,
      NOW,
    );
    const byDevice = new Map(rows.map((r) => [r.deviceName, r]));
    expect(byDevice.get("AYESHA-PC")?.outdated).toBe(false);
    expect(byDevice.get("TANVIR-PC")?.outdated).toBe(true);
    // An agent that reports no version can't be compared, so it isn't flagged.
    expect(byDevice.get("OLD")?.outdated).toBe(false);
  });

  it("keeps devices that only uploaded (agents too old for heartbeats) with no live status", () => {
    const [row] = buildDeviceRows([], [reportRow({ deviceId: null, deviceName: null })], name, NOW);
    expect(row.deviceId).toBeNull();
    expect(row.status).toBeNull();
    expect(row.online).toBe(false);
    expect(deviceStatusText(row, NOW)).toBe("No live status · last upload 10m ago");
  });

  it("ages a heartbeat with the page's clock: past 150 s the device is offline", () => {
    const [row] = buildDeviceRows([presence({ lastSeenAt: ago(151), online: true })], [], name, NOW);
    expect(row.online).toBe(false);
    expect(deviceStatusText(row, NOW)).toBe("Offline · last seen 2m ago");
    expect(deviceStatusTone(row)).toBe("offline");
  });

  it("lists online devices first, then the most recently seen", () => {
    const rows = buildDeviceRows(
      [presence({ userId: "u2", deviceId: "d2", lastSeenAt: ago(5000), online: false }), presence()],
      [reportRow({ userId: "u3", deviceId: "d3", lastActivityAt: ago(60) })],
      name,
      NOW,
    );
    expect(rows.map((r) => r.userId)).toEqual(["u1", "u3", "u2"]);
  });

  it("sums a member's unknown-device rows and ignores malformed ones", () => {
    const rows = buildDeviceRows(
      [{ ...presence(), deviceId: "" } as PresenceDto, null as unknown as PresenceDto],
      [reportRow({ deviceId: null, trackedMinutes: 30 }), reportRow({ deviceId: null, trackedMinutes: 15.4 }), { userId: 5 } as unknown as DeviceReportRowDto],
      name,
      NOW,
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].trackedMinutes).toBe(45);
  });
});

describe("status words", () => {
  it("reads the heartbeat status while online", () => {
    const [row] = buildDeviceRows([presence({ status: "break" })], [], name, NOW);
    expect(deviceStatusText(row, NOW)).toBe("On a break");
    expect(deviceStatusTone(row)).toBe("away");
  });

  it("names the OS as reported, else the platform", () => {
    expect(osText({ platform: "macos", osVersion: null })).toBe("macOS");
    expect(osText({ platform: "linux", osVersion: null })).toBe("Linux");
    expect(osText({ platform: null, osVersion: null })).toBe("Unknown OS");
    expect(osText({ platform: "windows", osVersion: "Windows 10" })).toBe("Windows 10");
  });
});

describe("memberPresence", () => {
  it("takes each member's most active device", () => {
    const map = memberPresence(
      [
        presence({ deviceId: "a", status: "paused" }),
        presence({ deviceId: "b", status: "tracking" }),
        presence({ userId: "u2", deviceId: "c", status: "locked" }),
        presence({ userId: "u3", deviceId: "d", status: "tracking", lastSeenAt: ago(600), online: false }),
        presence({ userId: "u4", deviceId: "e", status: "signedout" }),
      ],
      NOW,
    );
    expect(map.get("u1")).toBe("active");
    expect(map.get("u2")).toBe("idle");
    expect(map.get("u3")).toBe("offline");
    expect(map.get("u4")).toBe("offline");
    expect(map.has("u5")).toBe(false);
  });
});

describe("demoDeviceSources", () => {
  it("gives each tracked demo member one device, from their own demo rows — never clients", () => {
    const users = [
      buildUser({ id: "a", name: "Ana Ito", status: "active" }),
      buildUser({ id: "b", name: "Bo Li", status: "offline" }),
      buildUser({ id: "c", name: "Cy Client", role: "client" }),
    ];
    const activities = [
      buildActivity({ userId: "a", startedAt: "2026-10-05T10:00:00Z", endedAt: "2026-10-05T10:30:00Z" }),
      buildActivity({ userId: "b", startedAt: "2026-10-01T09:00:00Z", endedAt: "2026-10-01T09:10:00Z" }),
    ];
    const { presence: hb, report } = demoDeviceSources(users, activities, NOW);
    expect(report.map((r) => r.userId).sort()).toEqual(["a", "b"]);
    expect(report.find((r) => r.userId === "a")?.trackedMinutes).toBe(30);
    expect(hb.find((p) => p.userId === "a")?.status).toBe("tracking");
    expect(hb.find((p) => p.userId === "b")?.online).toBe(false);
    expect([...hb, ...report].some((r) => r.userId === "c")).toBe(false);
  });
});
