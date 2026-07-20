import { projects } from "./tenant-data";
import { isTrackedMember } from "./roles";
import type { Activity, Project, User } from "./types";

export { isTrackedMember, isSeatMember, trackedMembers, countSeats, inviteableRoles } from "./roles";

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
export function scopeProjects(user: User, list: Project[] = projects): Project[] {
  if (user.role === "owner" || user.role === "admin") return list;
  return list.filter((p) => p.memberIds.includes(user.id));
}

/** Whether the viewer may open a project detail page. */
export function canViewProject(user: User, project: Project): boolean {
  if (user.role === "owner" || user.role === "admin") return true;
  return project.memberIds.includes(user.id);
}
