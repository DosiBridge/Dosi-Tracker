// TeamPage invite behavior in demo mode: who gets the invite affordance and
// exactly which roles each inviter may assign — verified against the
// inviteableRoles matrix in src/lib/roles.ts, not against a copy of it.
//
// Email-format validation is enforced before anything is sent (in BOTH demo
// and live paths) — the "rejects malformed addresses" tests below were written
// against the unvalidated implementation, observed failing, and now pin the fix.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import TeamPage from "@/app/(dashboard)/team/page";
import { renderAsRole, resetPrototypeState } from "@/test/harness";
import { inviteableRoles } from "@/lib/roles";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetPrototypeState();
  // Demo mode (no dosi-token) must never touch the network; a never-settling
  // stub makes any accidental request hang the assertion instead of passing.
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openInviteModal(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /invite member/i }));
  return screen.getByRole("dialog");
}

/** The invite dialog has two selects (Role, Project); the Role one is the one offering "Member". */
function roleSelect(dialog: HTMLElement): HTMLSelectElement {
  const match = within(dialog)
    .getAllByRole("combobox")
    .find((s) => within(s).queryByRole("option", { name: "Member" }));
  if (!(match instanceof HTMLSelectElement)) throw new Error("role select not found in invite dialog");
  return match;
}

describe("team page — invite permissions", () => {
  it("owner can invite and is offered exactly worker/admin/client (never owner)", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    const options = within(roleSelect(dialog)).getAllByRole("option");

    expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual(
      inviteableRoles({ role: "owner" }), // ["worker", "admin", "client"]
    );
    expect(options.map((o) => o.textContent?.trim())).toEqual(["Member", "Admin", "Client"]);
    expect(within(dialog).queryByRole("option", { name: /owner/i })).not.toBeInTheDocument();
  });

  it("admin can invite but is never offered the Admin or Owner role", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "admin");

    const dialog = await openInviteModal(user);
    const options = within(roleSelect(dialog)).getAllByRole("option");

    expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual(
      inviteableRoles({ role: "admin" }), // ["worker", "client"]
    );
    expect(within(dialog).queryByRole("option", { name: /admin/i })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("option", { name: /owner/i })).not.toBeInTheDocument();
  });

  it("a worker gets no invite affordance at all", () => {
    renderAsRole(<TeamPage />, "worker");
    expect(screen.queryByRole("button", { name: /invite member/i })).not.toBeInTheDocument();
  });

  it("a client gets no invite affordance at all", () => {
    renderAsRole(<TeamPage />, "client");
    expect(screen.queryByRole("button", { name: /invite member/i })).not.toBeInTheDocument();
  });
});

describe("team page — sending invites (demo mode)", () => {
  it("send stays disabled until at least one email is entered", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    expect(within(dialog).getByRole("button", { name: /send invites/i })).toBeDisabled();
  });

  it("submitting emails shows the sent confirmation without hitting the network", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByPlaceholderText("jane@company.com, john@company.com"),
      "jane@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await screen.findByText("Invitations sent!")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects a malformed address, names the offender, and sends nothing", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "not-an-email",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await within(dialog).findByText(/not a valid email address: not-an-email/i)).toBeInTheDocument();
    expect(screen.queryByText("Invitations sent!")).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects a batch when only one address of several is malformed", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "jane@company.dev, broken@, john@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await within(dialog).findByText(/broken@/)).toBeInTheDocument();
    expect(screen.queryByText("Invitations sent!")).not.toBeInTheDocument();
  });

  it("accepts a comma-separated batch of valid addresses", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "jane@company.dev, john@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await screen.findByText("Invitations sent!")).toBeInTheDocument();
  });

  it("labels every invite field for assistive tech", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    expect(within(dialog).getByLabelText(/email addresses/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^role$/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^project$/i)).toBeInTheDocument();
  });
});
