"use client";

import { useId, useState } from "react";
import { Check, ChevronRight, ChevronLeft } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Avatar } from "@/components/ui/avatar";
import { users } from "@/lib/tenant-data";
import { PROJECT_LIMITS, type CreateUpdateProjectDto, type NewProjectDefaultsDto } from "@/lib/api-contract";
import { cn } from "@/lib/utils";
import {
  BUILT_IN_PROJECT_DEFAULTS,
  PROJECT_SWATCHES,
  normalizeNewProjectDefaults,
  toProjectDto,
  validateProjectDraft,
  type ProjectDraft,
  type ProjectDraftErrors,
} from "./project-form";
import { FieldError, TrackingFields } from "./tracking-fields";

// The tracking fields live on their own (Settings uses them without the dialogs).
export { TrackingFields };

/* ============================================================================
 * Create and edit a project. Creating is a three-step wizard (details, team,
 * tracking) that starts from the workspace's new-project defaults; editing is
 * one form with the same fields. Both submit a CreateUpdateProjectDto and stay
 * open with the server's message when the save fails.
 *
 * Render these only while open (`{open && <CreateProjectModal … />}`): every
 * opening then starts from a clean draft.
 * ========================================================================== */

export interface ProjectFormResult {
  dto: CreateUpdateProjectDto;
  /** Create only: the people picked on the Team step (added as members after the project exists). */
  memberIds: string[];
}

type Submit = (result: ProjectFormResult) => void | Promise<void>;

const steps = ["Details", "Team", "Tracking"] as const;

type Details = Pick<ProjectDraft, "title" | "description" | "color">;

/* ----------------------------- Fields ----------------------------- */

function DetailsFields({
  draft,
  errors,
  showErrors,
  onChange,
}: {
  draft: ProjectDraft;
  errors: ProjectDraftErrors;
  showErrors: boolean;
  onChange: (patch: Partial<Details>) => void;
}) {
  const id = useId();
  const swatchName = `${id}-colour`;
  const custom = !PROJECT_SWATCHES.some((s) => s.value.toLowerCase() === draft.color.toLowerCase());
  const titleError = showErrors ? errors.title : undefined;
  const descriptionLength = draft.description.trim().length;
  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label htmlFor={`${id}-title`} className="text-sm font-medium">
          Project name
        </label>
        <Input
          id={`${id}-title`}
          value={draft.title}
          onChange={(e) => onChange({ title: e.target.value })}
          placeholder="e.g. Mobile App Revamp"
          maxLength={PROJECT_LIMITS.titleMaxLength}
          aria-invalid={titleError ? true : undefined}
          aria-describedby={titleError ? `${id}-title-error` : undefined}
          autoComplete="off"
        />
        <FieldError id={`${id}-title-error`} message={titleError} />
      </div>
      <div className="space-y-1.5">
        <label htmlFor={`${id}-description`} className="text-sm font-medium">
          Description <span className="font-normal text-muted-foreground">(optional)</span>
        </label>
        <textarea
          id={`${id}-description`}
          value={draft.description}
          onChange={(e) => onChange({ description: e.target.value })}
          placeholder="Short summary"
          rows={3}
          maxLength={PROJECT_LIMITS.descriptionMaxLength}
          aria-invalid={errors.description ? true : undefined}
          aria-describedby={`${id}-description-count`}
          className="w-full resize-y rounded-xl border border-input bg-card px-3 py-2 text-sm transition-shadow placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
        />
        <p id={`${id}-description-count`} className="text-right text-[11px] tabular-nums text-muted-foreground">
          {descriptionLength.toLocaleString("en")} / {PROJECT_LIMITS.descriptionMaxLength.toLocaleString("en")}
        </p>
        <FieldError id={`${id}-description-error`} message={errors.description} />
      </div>
      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">Colour</legend>
        <div className="flex flex-wrap items-center gap-2.5">
          {PROJECT_SWATCHES.map((s) => {
            const checked = s.value.toLowerCase() === draft.color.toLowerCase();
            return (
              <label
                key={s.value}
                title={s.name}
                className={cn(
                  "relative flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-transform motion-reduce:transition-none",
                  "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-card",
                  checked && "scale-110 ring-2 ring-foreground/70 ring-offset-2 ring-offset-card",
                )}
                style={{ background: s.value }}
              >
                <input
                  type="radio"
                  name={swatchName}
                  value={s.value}
                  checked={checked}
                  onChange={() => onChange({ color: s.value })}
                  aria-label={s.name}
                  className="sr-only"
                />
                {checked && <Check aria-hidden className="h-4 w-4 text-white" />}
              </label>
            );
          })}
          <label
            className={cn(
              "inline-flex h-8 cursor-pointer items-center gap-2 rounded-full border px-2.5 text-xs font-medium",
              "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
              custom ? "border-foreground/60 text-foreground" : "border-border text-muted-foreground",
            )}
          >
            <input
              type="color"
              value={/^#[0-9a-fA-F]{6}$/.test(draft.color) ? draft.color : PROJECT_SWATCHES[0].value}
              onChange={(e) => onChange({ color: e.target.value })}
              className="h-5 w-5 cursor-pointer rounded-full border-0 bg-transparent p-0"
            />
            Custom
          </label>
        </div>
        <FieldError id={`${id}-colour-error`} message={showErrors ? errors.color : undefined} />
      </fieldset>
    </div>
  );
}

/* ----------------------------- Create ----------------------------- */

export function CreateProjectModal({
  onClose,
  onCreate,
  defaults,
  defaultsNote,
  currentUserId,
}: {
  onClose: () => void;
  onCreate: Submit;
  /** The workspace's new-project defaults; null or absent uses the built-in ones. */
  defaults?: NewProjectDefaultsDto | null;
  /** Where the tracking defaults came from, shown on the Tracking step. */
  defaultsNote?: React.ReactNode;
  /** The creator (joins as the project's administrator automatically, so not offered on the Team step). */
  currentUserId?: string;
}) {
  const [step, setStep] = useState(0);
  const [details, setDetails] = useState<Details>({ title: "", description: "", color: PROJECT_SWATCHES[0].value });
  // Only what the creator changed: the rest follows the workspace defaults, even if they load late.
  const [trackingEdits, setTrackingEdits] = useState<Partial<NewProjectDefaultsDto>>({});
  const [members, setMembers] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const draft: ProjectDraft = {
    ...details,
    ...normalizeNewProjectDefaults(defaults ?? BUILT_IN_PROJECT_DEFAULTS),
    ...trackingEdits,
  };
  const errors = validateProjectDraft(draft);
  const people = users.filter((u) => u.role !== "client" && u.id !== currentUserId);
  const canNext = step === 0 ? !errors.title && !errors.description && !errors.color : true;

  async function finish() {
    if (Object.keys(errors).length > 0) {
      setError("Check the highlighted fields.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onCreate({ dto: toProjectDto(draft), memberIds: members });
      onClose();
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't create the project. Try again.");
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={onClose} title="Create project" description="Set up tracking for a new project.">
      {/* Stepper */}
      <ol className="mb-6 flex items-center" aria-label="Steps">
        {steps.map((s, i) => (
          <li key={s} className="flex flex-1 items-center last:flex-none" aria-current={i === step ? "step" : undefined}>
            <div className="flex items-center gap-2">
              <div
                aria-hidden
                className={cn(
                  "flex h-7 w-7 items-center justify-center rounded-full text-xs font-semibold transition-colors",
                  i < step ? "bg-primary text-primary-foreground" : i === step ? "bg-primary/15 text-primary-strong ring-2 ring-primary" : "bg-muted text-muted-foreground",
                )}
              >
                {i < step ? <Check className="h-3.5 w-3.5" /> : i + 1}
              </div>
              <span className={cn("text-xs font-medium", i === step ? "text-foreground" : "text-muted-foreground")}>
                <span className="sr-only">Step {i + 1}: </span>
                {s}
              </span>
            </div>
            {i < steps.length - 1 && <div aria-hidden className={cn("mx-2 h-px flex-1", i < step ? "bg-primary" : "bg-border")} />}
          </li>
        ))}
      </ol>

      {step === 0 && (
        <DetailsFields
          draft={draft}
          errors={errors}
          showErrors={details.title.length > 0}
          onChange={(patch) => setDetails((d) => ({ ...d, ...patch }))}
        />
      )}

      {step === 1 && (
        <fieldset className="space-y-2">
          <legend className="text-sm text-muted-foreground">Pick the people who will track time on this project.</legend>
          {currentUserId && <p className="text-xs text-muted-foreground">You join automatically as the project&apos;s administrator.</p>}
          {people.length === 0 ? (
            <p className="rounded-xl border border-dashed border-border px-3 py-6 text-center text-sm text-muted-foreground">
              No one else is in the workspace yet. Invite people from the Team page after creating the project.
            </p>
          ) : (
            <div className="max-h-72 space-y-1 overflow-y-auto">
              {people.map((u) => {
                const selected = members.includes(u.id);
                return (
                  <label
                    key={u.id}
                    className={cn(
                      "flex w-full cursor-pointer items-center gap-3 rounded-lg border p-2.5 text-left transition-colors",
                      "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                      selected ? "border-primary bg-primary/5" : "border-border hover:bg-muted/60",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={selected}
                      onChange={() => setMembers((m) => (selected ? m.filter((x) => x !== u.id) : [...m, u.id]))}
                      className="sr-only"
                    />
                    <Avatar name={u.name} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">{u.name}</span>
                      <span className="block truncate text-xs text-muted-foreground">{u.email || u.designation}</span>
                    </span>
                    <span
                      aria-hidden
                      className={cn(
                        "flex h-5 w-5 items-center justify-center rounded-full border",
                        selected ? "border-primary bg-primary text-primary-foreground" : "border-border",
                      )}
                    >
                      {selected && <Check className="h-3 w-3" />}
                    </span>
                  </label>
                );
              })}
            </div>
          )}
        </fieldset>
      )}

      {step === 2 && (
        <TrackingFields
          draft={draft}
          errors={errors}
          defaultsNote={defaultsNote}
          onChange={(patch) => setTrackingEdits((t) => ({ ...t, ...patch }))}
        />
      )}

      {error && (
        <p role="alert" className="mt-4 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
          {error}
        </p>
      )}

      {/* Footer */}
      <div className="mt-6 flex items-center justify-between gap-2">
        <Button variant="ghost" onClick={step === 0 ? onClose : () => setStep((s) => s - 1)} disabled={busy}>
          {step === 0 ? (
            "Cancel"
          ) : (
            <>
              <ChevronLeft className="h-4 w-4" /> Back
            </>
          )}
        </Button>
        {step < steps.length - 1 ? (
          <Button onClick={() => setStep((s) => s + 1)} disabled={!canNext}>
            Continue <ChevronRight className="h-4 w-4" />
          </Button>
        ) : (
          <Button onClick={() => void finish()} disabled={busy}>
            <Check className="h-4 w-4" /> {busy ? "Creating…" : "Create project"}
          </Button>
        )}
      </div>
    </Modal>
  );
}

/* ----------------------------- Edit ----------------------------- */

export function EditProjectModal({
  initial,
  onClose,
  onSave,
  note,
}: {
  /** The project as stored (see draftFromProject / draftFromDemoProject). */
  initial: ProjectDraft;
  onClose: () => void;
  onSave: Submit;
  /** A line under the form (e.g. that a demo edit isn't saved). */
  note?: React.ReactNode;
}) {
  const [draft, setDraft] = useState<ProjectDraft>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"details" | "tracking">("details");
  const tabsId = useId();
  const errors = validateProjectDraft(draft);
  const detailsInvalid = !!(errors.title || errors.description || errors.color);

  async function save() {
    if (Object.keys(errors).length > 0) {
      setTab(detailsInvalid ? "details" : "tracking");
      setError("Check the highlighted fields.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onSave({ dto: toProjectDto(draft), memberIds: [] });
      onClose();
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't save the project. Try again.");
      setBusy(false);
    }
  }

  const tabs = [
    { key: "details" as const, label: "Details" },
    { key: "tracking" as const, label: "Tracking" },
  ];

  // Arrow keys move between the two tabs (WAI-ARIA tabs); only the selected one is in the Tab order.
  function onTabKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight" && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const index = tabs.findIndex((t) => t.key === tab);
    const next =
      event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    setTab(tabs[next].key);
    document.getElementById(`${tabsId}-${tabs[next].key}-tab`)?.focus();
  }

  return (
    <Modal
      open
      onClose={onClose}
      title="Edit project"
      description="Tracking changes reach the desktop agents within five minutes and apply from their next block."
    >
      <div role="tablist" aria-label="Project settings" onKeyDown={onTabKeyDown} className="mb-5 inline-flex rounded-xl bg-muted p-1">
        {tabs.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            id={`${tabsId}-${t.key}-tab`}
            aria-selected={tab === t.key}
            aria-controls={tab === t.key ? `${tabsId}-${t.key}-panel` : undefined}
            tabIndex={tab === t.key ? 0 : -1}
            onClick={() => setTab(t.key)}
            className={cn(
              "inline-flex h-8 items-center rounded-lg px-3 text-xs font-medium transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              // Muted text on the muted track falls under 4.5:1; a softened foreground clears it in both themes.
              tab === t.key ? "bg-card text-foreground shadow-sm" : "text-foreground/75 hover:text-foreground",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`${tabsId}-${tab}-panel`} aria-labelledby={`${tabsId}-${tab}-tab`}>
        {tab === "details" ? (
          <DetailsFields draft={draft} errors={errors} showErrors onChange={(patch) => setDraft((d) => ({ ...d, ...patch }))} />
        ) : (
          <TrackingFields draft={draft} errors={errors} onChange={(patch) => setDraft((d) => ({ ...d, ...patch }))} />
        )}
      </div>

      {note && <p className="mt-4 text-xs text-muted-foreground">{note}</p>}
      {error && (
        <p role="alert" className="mt-4 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
          {error}
        </p>
      )}

      <div className="mt-6 flex items-center justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button onClick={() => void save()} disabled={busy}>
          {busy ? "Saving…" : "Save changes"}
        </Button>
      </div>
    </Modal>
  );
}
