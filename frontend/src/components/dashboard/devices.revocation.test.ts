// Device revocation in the devices list (E2E-09): the registry (GET
// /api/app/device) marks revoked installs — keeping one presence no longer
// lists — and decides who may revoke or restore what.
import { describe, expect, it } from "vitest";
import type { PresenceDto, WorkspaceDeviceDto } from "@/lib/api-contract";
import { buildDeviceRows, deviceActions, deviceStatusText, deviceStatusTone } from "./devices";

const NOW = new Date("2026-10-05T12:00:00Z");
const ago = (seconds: number) => new Date(NOW.getTime() - seconds * 1000).toISOString();
const name = (userId: string, hint: string | null) => ({ u1: "Ayesha Rahman", u2: "Tanvir Hasan" })[userId] ?? hint ?? "Team member";

const live: PresenceDto = {
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
};

function device(patch: Partial<WorkspaceDeviceDto> = {}): WorkspaceDeviceDto {
  return { userId: "u1", deviceId: "d1", isRevoked: false, ...patch };
}

describe("buildDeviceRows with the device registry", () => {
  it("marks a revoked install offline whatever its last heartbeat said, with when and why", () => {
    const [row] = buildDeviceRows([live], [], name, NOW, [device({ isRevoked: true, revokedAt: "2026-10-05T11:00:00", reason: "Laptop lost" })]);
    expect(row).toMatchObject({ revoked: true, online: false, revokedAt: "2026-10-05T11:00:00.000Z", revokeReason: "Laptop lost", deviceName: "AYESHA-PC" });
    expect(deviceStatusText(row, NOW)).toBe("Revoked · signed out");
    expect(deviceStatusTone(row)).toBe("offline");
  });

  it("keeps an install that only the registry still knows, with its last descriptors", () => {
    const rows = buildDeviceRows([], [], name, NOW, [
      device({ userId: "u2", deviceId: "d9", deviceName: "TANVIR-MAC", platform: "macos", agentVersion: "0.1.0", lastSeenAt: "2026-10-01T09:00:00Z", isRevoked: true }),
    ]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ memberName: "Tanvir Hasan", deviceName: "TANVIR-MAC", platform: "macos", revoked: true, lastSeenAt: "2026-10-01T09:00:00.000Z" });
  });

  it("leaves a device that isn't revoked as presence says, and ignores malformed rows", () => {
    const rows = buildDeviceRows([live], [], name, NOW, [device(), { userId: 7 } as unknown as WorkspaceDeviceDto, device({ deviceId: " " })]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ revoked: false, online: true, revokedAt: null, revokeReason: null });
  });

  it("defaults to no registry (an older server): nothing is revoked", () => {
    expect(buildDeviceRows([live], [], name, NOW)[0].revoked).toBe(false);
  });
});

describe("deviceActions — who may revoke or restore", () => {
  const base = { viewerId: "m", rowUserId: "u", rowUserIsOwner: false, revoked: false, hasDeviceId: true, viewerIsOwner: false, canManageDevices: true };

  it("lets owners and managers revoke a member's device, and restore it once revoked", () => {
    expect(deviceActions(base)).toEqual({ revoke: true, restore: false });
    expect(deviceActions({ ...base, revoked: true })).toEqual({ revoke: false, restore: true });
  });

  it("leaves an owner's devices to owners", () => {
    expect(deviceActions({ ...base, rowUserIsOwner: true })).toEqual({ revoke: false, restore: false });
    expect(deviceActions({ ...base, rowUserIsOwner: true, viewerIsOwner: true })).toEqual({ revoke: true, restore: false });
  });

  it("lets a member revoke only their own device, and never restore", () => {
    const member = { ...base, canManageDevices: false };
    expect(deviceActions(member)).toEqual({ revoke: false, restore: false });
    expect(deviceActions({ ...member, rowUserId: "m" })).toEqual({ revoke: true, restore: false });
    expect(deviceActions({ ...member, rowUserId: "m", revoked: true })).toEqual({ revoke: false, restore: false });
  });

  it("offers nothing for an install without a device id (too old to be revoked)", () => {
    expect(deviceActions({ ...base, hasDeviceId: false })).toEqual({ revoke: false, restore: false });
  });
});
