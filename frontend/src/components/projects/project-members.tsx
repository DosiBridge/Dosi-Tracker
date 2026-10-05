"use client";

import { useId, useMemo, useState } from "react";
import { Search, UserPlus } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Avatar } from "@/components/ui/avatar";
import type { CreateProjectMemberDto, MembershipEditRules, ProjectRole } from "@/lib/api-contract";
import { cn } from "@/lib/utils";
import { PROJECT_ROLE_HINTS, PROJECT_ROLE_LABELS, membershipErrorText, parseHourlyRate, roleChoices } from "./membership";

/* ============================================================================
 * Add an existing workspace member to a project (POST /api/app/team/member).
 * Used from the Team page (pick the person, then the project) and from a
 * project's page (the project is fixed; pick the person). People already on
 * the chosen project can't be picked, and the role and rate fields follow
 * the viewer's rights (only owners grant Administrator or set rates).
 * ========================================================================== */

export interface PickablePerson {
  id: string;
  name: string;
  email: string;
}

export interface PickableProject {
  id: string;
  title: string;
}

export function AddMemberDialog({
  people,
  projects,
  fixedProjectId,
  initialPersonId,
  memberIdsOf,
  rulesFor,
  onClose,
  onSubmit,
}: {
  /** The workspace's people (the viewer excluded: nobody changes their own memberships). */
  people: readonly PickablePerson[];
  /** Active projects to choose from (ignored when `fixedProjectId` is set). */
  projects: readonly PickableProject[];
  /** Add to this project only (a project's own page). */
  fixedProjectId?: string | null;
  /** Start with this person picked (a member's card on the Team page). */
  initialPersonId?: string | null;
  /** Who is already on a project; null while unknown. */
  memberIdsOf: (projectId: string) => ReadonlySet<string> | null;
  /** What the viewer may grant to a given person. */
  rulesFor: (personId: string) => MembershipEditRules;
  onClose: () => void;
  /** Sends the membership; rejects with the server's error. */
  onSubmit: (input: CreateProjectMemberDto) => Promise<void>;
}) {
  const id = useId();
  const [query, setQuery] = useState("");
  const [personId, setPersonId] = useState<string>(initialPersonId ?? "");
  const [projectId, setProjectId] = useState<string>(fixedProjectId ?? "");
  const [role, setRole] = useState<ProjectRole>("Worker");
  const [rate, setRate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fixedMembers = fixedProjectId ? memberIdsOf(fixedProjectId) : null;
  const visiblePeople = useMemo(() => {
    const q = query.trim().toLowerCase();
    return [...people]
      .filter((p) => !q || p.name.toLowerCase().includes(q) || p.email.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [people, query]);
  const projectChoices = useMemo(() => [...projects].sort((a, b) => a.title.localeCompare(b.title)), [projects]);

  const rules = personId ? rulesFor(personId) : null;
  const roles = rules ? roleChoices(rules) : [];
  const chosenRole = roles.includes(role) ? role : (roles[0] ?? "Worker");
  const alreadyOnProject = !!personId && !!projectId && !!memberIdsOf(projectId)?.has(personId);
  const parsedRate = rules?.canChangeRate ? parseHourlyRate(rate) : 0;
  const person = people.find((p) => p.id === personId) ?? null;

  async function submit() {
    if (!personId) return setError("Pick a person.");
    if (!projectId) return setError("Pick a project.");
    if (alreadyOnProject) return setError("They're already on this project.");
    if (roles.length === 0) return setError("You can't change this person's projects.");
    if (parsedRate === null) return setError("Hourly rate must be a number from 0 to 10,000 with at most two decimals.");
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ projectId, userId: personId, role: chosenRole, hourlyRate: parsedRate });
      onClose();
    } catch (err) {
      setError(membershipErrorText(err, "Couldn't add them to the project."));
      setBusy(false);
    }
  }

  return (
    <Modal
      open
      onClose={onClose}
      title="Add to a project"
      description="Give someone already in the workspace a place on a project. Their desktop agent offers it within five minutes."
    >
      <div className="space-y-4">
        <fieldset className="space-y-2">
          <legend className="text-sm font-medium">Person</legend>
          <div className="relative">
            <Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search by name or email"
              aria-label="Search people"
              className="pl-9"
            />
          </div>
          {people.length === 0 ? (
            <p className="rounded-xl border border-dashed border-border px-3 py-5 text-center text-sm text-muted-foreground">
              No one else is in the workspace yet — invite them by email instead.
            </p>
          ) : visiblePeople.length === 0 ? (
            <p className="px-1 py-3 text-sm text-muted-foreground">No one matches “{query}”.</p>
          ) : (
            <div className="max-h-56 space-y-1 overflow-y-auto pr-1">
              {visiblePeople.map((p) => {
                const taken = !!fixedMembers?.has(p.id);
                const checked = personId === p.id;
                return (
                  <label
                    key={p.id}
                    className={cn(
                      "flex items-center gap-3 rounded-lg border p-2.5 transition-colors",
                      "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                      taken ? "cursor-not-allowed border-border opacity-60" : "cursor-pointer",
                      checked ? "border-primary bg-primary/5" : !taken && "border-border hover:bg-muted/60",
                    )}
                  >
                    <input
                      type="radio"
                      name={`${id}-person`}
                      value={p.id}
                      checked={checked}
                      disabled={taken}
                      onChange={() => {
                        setPersonId(p.id);
                        setError(null);
                      }}
                      className="sr-only"
                    />
                    <Avatar name={p.name} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">{p.name}</span>
                      <span className="block truncate text-xs text-muted-foreground">{p.email || "No email on file"}</span>
                    </span>
                    {taken && <span className="shrink-0 text-xs text-muted-foreground">Already on the project</span>}
                  </label>
                );
              })}
            </div>
          )}
        </fieldset>

        {!fixedProjectId && (
          <div className="space-y-1.5">
            <label htmlFor={`${id}-project`} className="text-sm font-medium">
              Project
            </label>
            <Select id={`${id}-project`} value={projectId} onChange={(e) => setProjectId(e.target.value)}>
              <option value="">Choose a project…</option>
              {projectChoices.map((p) => {
                const on = !!personId && !!memberIdsOf(p.id)?.has(personId);
                return (
                  <option key={p.id} value={p.id} disabled={on}>
                    {on ? `${p.title} — already a member` : p.title}
                  </option>
                );
              })}
            </Select>
            {projectChoices.length === 0 && <p className="text-xs text-muted-foreground">There are no active projects yet.</p>}
          </div>
        )}

        <div className={cn("grid gap-3", rules?.canChangeRate ? "sm:grid-cols-2" : "")}>
          <div className="space-y-1.5">
            <label htmlFor={`${id}-role`} className="text-sm font-medium">
              Role
            </label>
            <Select
              id={`${id}-role`}
              value={chosenRole}
              onChange={(e) => setRole(e.target.value as ProjectRole)}
              disabled={roles.length === 0}
              aria-describedby={`${id}-role-hint`}
            >
              {roles.map((r) => (
                <option key={r} value={r}>
                  {PROJECT_ROLE_LABELS[r]}
                </option>
              ))}
            </Select>
            <p id={`${id}-role-hint`} className="text-xs text-muted-foreground">
              {PROJECT_ROLE_HINTS[chosenRole]}
            </p>
          </div>
          {rules?.canChangeRate && (
            <div className="space-y-1.5">
              <label htmlFor={`${id}-rate`} className="text-sm font-medium">
                Hourly rate (USD) <span className="font-normal text-muted-foreground">(optional)</span>
              </label>
              <Input
                id={`${id}-rate`}
                inputMode="decimal"
                value={rate}
                onChange={(e) => setRate(e.target.value)}
                placeholder="0.00"
              />
            </div>
          )}
        </div>

        {person && projectId && !alreadyOnProject && (
          <p className="text-xs text-muted-foreground">
            {person.name} will see the project in the desktop agent. Seats are counted per person, so this uses no extra seat if
            they&apos;re already on another project.
          </p>
        )}
        {error && (
          <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={busy || !personId || !projectId || alreadyOnProject}>
            <UserPlus className="h-4 w-4" /> {busy ? "Adding…" : "Add to project"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
