import {
  PROJECT_ROLES,
  membershipEditRules,
  type MembershipEditRules,
  type ProjectMemberDto,
  type ProjectRole,
} from "@/lib/api-contract";
import { displayName } from "@/lib/live-dataset";
import { roleLabels } from "@/lib/roles";
import type { Role } from "@/lib/types";
import { requestErrorCode, requestErrorText } from "@/components/layout/request-error";

/* ============================================================================
 * PROJECT MEMBERSHIP — the rules the Team page and a project's page share for
 * adding, editing and removing someone's place on a project (contract §7).
 *
 * The server enforces every rule; the UI mirrors them so nobody is offered a
 * change that would only come back as a 403: only workspace owners grant the
 * Admin/Manager roles or change hourly rates, and nobody changes their own.
 * Role names read the same everywhere (roleLabels): Worker is "Member" and
 * Admin is "Administrator".
 * ========================================================================== */

export const PROJECT_ROLE_LABELS: Record<ProjectRole, string> = {
  Admin: roleLabels.admin,
  Manager: "Manager",
  Worker: roleLabels.worker,
  Client: roleLabels.client,
};

/** One line per role, for the help text under a role picker. */
export const PROJECT_ROLE_HINTS: Record<ProjectRole, string> = {
  Admin: "Manages the project and its people, and sees everyone's activity.",
  Manager: "Manages the project and its people, and sees everyone's activity.",
  Worker: "Tracks time on the project with the desktop agent.",
  Client: "Follows the project's progress. Never tracked.",
};

/** The order roles are offered in: the everyday choice first. */
const OFFER_ORDER: readonly ProjectRole[] = ["Worker", "Admin", "Client"];

/** A membership's role from the server, read case-insensitively; anything unknown is a Worker. */
export function projectRoleOf(raw: string | null | undefined): ProjectRole {
  const wanted = (raw ?? "").trim().toLowerCase();
  return PROJECT_ROLES.find((r) => r.toLowerCase() === wanted) ?? "Worker";
}

/** "Administrator", "Manager", "Member" or "Client" for a membership's role. */
export function projectRoleLabel(raw: string | null | undefined): string {
  return PROJECT_ROLE_LABELS[projectRoleOf(raw)];
}

/** The app's role for a project role (Admin and Manager both manage). */
export function appRoleOf(role: ProjectRole): Role {
  if (role === "Admin" || role === "Manager") return "admin";
  if (role === "Client") return "client";
  return "worker";
}

/** What the signed-in user may change on a membership of `targetUserId`. */
export function editRulesFor(viewer: { id: string; role: Role }, targetUserId: string): MembershipEditRules {
  return membershipEditRules({
    canManageTeam: viewer.role === "owner" || viewer.role === "admin",
    callerIsOwner: viewer.role === "owner",
    isSelf: viewer.id === targetUserId,
  });
}

/**
 * The roles to offer in a picker, everyday choice first. Manager is not
 * offered for new grants (it is Administrator by another name); it is listed
 * only to show a membership that already holds it.
 */
export function roleChoices(rules: MembershipEditRules, current?: ProjectRole | null): ProjectRole[] {
  const offered = OFFER_ORDER.filter((r) => rules.roles.includes(r));
  if (current && !offered.includes(current)) offered.push(current);
  return offered;
}

/** A membership's person as shown in a list: their name from the roster or the membership, never a bare id. */
export function memberNameOf(
  member: Pick<ProjectMemberDto, "userId" | "userName" | "name" | "surname" | "email">,
  rosterName?: string | null,
): string {
  if (rosterName && rosterName.trim()) return rosterName.trim();
  return displayName({ name: member.name, surname: member.surname, userName: member.userName, email: member.email }, "Team member");
}

/** Hourly rate as typed: a number 0–10,000 with at most cents, or null when it isn't one. */
export function parseHourlyRate(raw: string): number | null {
  const text = raw.trim();
  if (text === "") return 0;
  if (!/^\d+(\.\d{1,2})?$/.test(text)) return null;
  const n = Number(text);
  return Number.isFinite(n) && n >= 0 && n <= 10_000 ? n : null;
}

/** "$25.00/h", or "No rate" for zero. */
export function rateLabel(rate: number | null | undefined): string {
  if (typeof rate !== "number" || !Number.isFinite(rate) || rate <= 0) return "No rate";
  return `$${rate.toFixed(2)}/h`;
}

/**
 * The sentence for a failed membership change. The business errors this flow
 * can meet get a sentence that says what to do next; anything else reads the
 * server's own message (requestErrorText).
 */
export function membershipErrorText(error: unknown, fallback: string): string {
  switch (requestErrorCode(error)) {
    case "Tracker:SeatLimitReached":
      return "Your plan has no free seat for another person. Upgrade the plan, or remove someone from the workspace first.";
    case "Tracker:DuplicateProjectMember":
      return "They're already on this project.";
    case "Tracker:InvalidUser":
      return "That person isn't in this workspace any more. Refresh the team list and try again.";
    case "Tracker:CannotModifySelf":
      return "You can't change your own role or hourly rate. Ask the workspace owner.";
    case "Tracker:RoleNotAllowed":
      return "Only the workspace owner can grant that role or change hourly rates.";
    default:
      return requestErrorText(error, fallback);
  }
}
