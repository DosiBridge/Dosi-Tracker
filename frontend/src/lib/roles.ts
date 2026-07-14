import type { Role } from "./types";

/** Which roles may access each route (by path prefix). */
export const navAccess: Record<string, Role[]> = {
  "/host": ["host"],
  "/dashboard": ["owner", "admin", "worker", "client"],
  "/projects": ["owner", "admin", "worker"],
  "/team": ["owner", "admin"],
  "/monitor": ["owner", "admin", "worker"],
  "/activities": ["owner", "admin", "worker", "client"],
  "/screenshots": ["owner", "admin", "worker", "client"],
  "/timesheet": ["owner", "admin", "worker"],
  "/reports": ["owner", "admin"],
  "/insights": ["owner", "admin"],
  "/billing": ["owner"],
  "/settings": ["owner", "admin", "worker", "client"],
};

export function canAccess(role: Role, href: string): boolean {
  const key = Object.keys(navAccess)
    .filter((k) => href === k || href.startsWith(k + "/"))
    .sort((a, b) => b.length - a.length)[0];
  if (!key) return true;
  return navAccess[key].includes(role);
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
  client: "Read-only view of your project's progress",
};
