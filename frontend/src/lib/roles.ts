import type { Role } from "./types";

/** Which roles may access each route (by path prefix). */
export const navAccess: Record<string, Role[]> = {
  "/host": ["host"],
  "/dashboard": ["owner", "admin", "worker", "client"],
  "/projects": ["owner", "admin", "worker"],
  "/team": ["owner", "admin"],
  "/monitor": ["owner", "admin", "worker"],
  // Capture review — not for clients (progress portal only)
  "/activities": ["owner", "admin", "worker"],
  // Legacy path redirects to /activities?view=screens
  "/screenshots": ["owner", "admin", "worker"],
  "/timesheet": ["owner", "admin", "worker"],
  // Clients see their projects' reports (the server scopes the data to those
  // projects, contract §6) — but only the ones in clientReportSlugs.
  "/reports": ["owner", "admin", "client"],
  // Insights demoted — page redirects to dashboard
  "/insights": ["owner", "admin"],
  "/billing": ["owner"],
  "/settings": ["owner", "admin", "worker", "client"],
  // Desktop agent downloads (onboarding) — anyone who is tracked
  "/download": ["owner", "admin", "worker"],
};

/**
 * The reports a client may open: time on the projects shared with them. Never
 * the ones built on members' activity, apps, captures, attendance or pay.
 */
export const clientReportSlugs: readonly string[] = ["work-report"];

const REPORTS_PREFIX = "/reports/";

export function canAccess(role: Role, href: string): boolean {
  const key = Object.keys(navAccess)
    .filter((k) => href === k || href.startsWith(k + "/"))
    .sort((a, b) => b.length - a.length)[0];
  if (!key) return true;
  if (!navAccess[key].includes(role)) return false;
  if (role === "client" && href.startsWith(REPORTS_PREFIX)) {
    const slug = href.slice(REPORTS_PREFIX.length).split(/[/?#]/)[0];
    return clientReportSlugs.includes(slug);
  }
  return true;
}

/** Where a role lands after login. */
export function landingFor(role: Role): string {
  return role === "host" ? "/host" : "/dashboard";
}

export const roleLabels: Record<Role, string> = {
  host: "Platform Admin",
  owner: "Owner",
  admin: "Administrator",
  worker: "Member",
  client: "Client",
};

export const roleDescriptions: Record<Role, string> = {
  host: "Operate the whole platform — tenants, plans & revenue",
  owner: "Full access — team, projects, reports & billing",
  admin: "Manage members, projects and monitoring",
  worker: "Your own activity, timesheet & projects",
  client: "Read-only progress and reports for the projects shared with you",
};

/**
 * Roles that occupy a paid seat and can be activity-tracked.
 * Clients are stakeholders who never track (the agents get no project for
 * them, contract §2); host is platform-only.
 */
export function isTrackedMember(user: { role: Role }): boolean {
  return user.role === "owner" || user.role === "admin" || user.role === "worker";
}

/** Alias for seat counting — same rule as tracked members. */
export function isSeatMember(user: { role: Role }): boolean {
  return isTrackedMember(user);
}

export function trackedMembers<T extends { role: Role }>(list: T[]): T[] {
  return list.filter(isTrackedMember);
}

export function countSeats(list: { role: Role }[]): number {
  return list.filter(isSeatMember).length;
}

/**
 * Roles the inviter may assign.
 * Admins cannot invite/promote to Owner; only owners can invite admins.
 */
export function inviteableRoles(inviter: { role: Role }): Role[] {
  if (inviter.role === "owner") return ["worker", "admin", "client"];
  if (inviter.role === "admin") return ["worker", "client"];
  return [];
}
