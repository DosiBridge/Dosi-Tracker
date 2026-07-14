import { projects } from "./tenant-data";
import type { Activity, User } from "./types";

/** Restrict an activity list to what a given role/user is allowed to see. */
export function scopeActivities(user: User, list: Activity[]): Activity[] {
  if (user.role === "worker") {
    return list.filter((a) => a.userId === user.id);
  }
  if (user.role === "client") {
    const myProjectIds = projects.filter((p) => p.memberIds.includes(user.id)).map((p) => p.id);
    return list.filter((a) => myProjectIds.includes(a.projectId));
  }
  return list; // owner / admin see everything
}

/** Projects a user is associated with (their own memberships). */
export function scopeProjects(user: User) {
  if (user.role === "owner" || user.role === "admin") return projects;
  return projects.filter((p) => p.memberIds.includes(user.id));
}
