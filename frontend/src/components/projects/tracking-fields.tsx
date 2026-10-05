"use client";

import { useId } from "react";
import { Info } from "lucide-react";
import { Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type { NewProjectDefaultsDto } from "@/lib/api-contract";
import { cn } from "@/lib/utils";
import { IDLE_CHOICES, INTERVAL_CHOICES, idleChoiceLabel, type PermissionField, type ProjectDraftErrors } from "./project-form";

/*
 * A project's tracking settings, shared by the project wizard and the
 * workspace's new-project defaults (Settings → Workspace). On its own so the
 * Settings page doesn't carry the project dialogs.
 */

const permissionRows: { key: PermissionField; label: string; desc: string }[] = [
  { key: "allowScreenshot", label: "Screenshots", desc: "One screen capture at a random moment in each block" },
  { key: "allowWebcam", label: "Webcam", desc: "A webcam photo with each block — only when the project truly needs it" },
  { key: "allowKeyboard", label: "Keyboard activity", desc: "Counts key presses for the activity % — never what is typed" },
  { key: "allowMouse", label: "Mouse activity", desc: "Counts clicks and movement for the activity %" },
  { key: "allowActiveWindow", label: "Active window", desc: "The focused app and its window title" },
  { key: "allowRunningPrograms", label: "Running programs", desc: "Names of open apps (never their window titles)" },
];

/** A field's validation message, linked from the field through aria-describedby. */
export function FieldError({ id, message }: { id: string; message?: string }) {
  if (!message) return null;
  return (
    <p id={id} className="text-xs text-danger-on-tint">
      {message}
    </p>
  );
}

/**
 * The tracking settings of a project (or of the workspace's new-project
 * defaults): block length, idle detection and what the agent captures.
 */
export function TrackingFields({
  draft,
  errors = {},
  onChange,
  defaultsNote,
}: {
  draft: NewProjectDefaultsDto;
  errors?: ProjectDraftErrors;
  onChange: (patch: Partial<NewProjectDefaultsDto>) => void;
  defaultsNote?: React.ReactNode;
}) {
  const id = useId();
  const intervals = INTERVAL_CHOICES.includes(draft.intervalMinutes)
    ? INTERVAL_CHOICES
    : [...INTERVAL_CHOICES, draft.intervalMinutes].sort((a, b) => a - b);
  const idles = IDLE_CHOICES.includes(draft.idleThresholdMinutes)
    ? IDLE_CHOICES
    : [...IDLE_CHOICES, draft.idleThresholdMinutes].sort((a, b) => a - b);
  return (
    <div className="space-y-5">
      {defaultsNote && (
        <p className="flex items-start gap-2 rounded-xl bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
          <Info aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{defaultsNote}</span>
        </p>
      )}
      <fieldset className="space-y-2" aria-describedby={`${id}-interval-hint`}>
        <legend className="text-sm font-medium">Block length</legend>
        <p id={`${id}-interval-hint`} className="text-xs text-muted-foreground">
          How long each tracked block lasts. At most one screenshot is taken in each block.
        </p>
        <div className="grid grid-cols-4 gap-2 sm:grid-cols-7">
          {intervals.map((v) => {
            const checked = draft.intervalMinutes === v;
            return (
              <label
                key={v}
                className={cn(
                  "flex h-10 cursor-pointer items-center justify-center rounded-lg border text-sm font-medium transition-colors",
                  "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                  checked ? "border-primary bg-primary/10 text-primary-strong" : "border-border hover:bg-muted/60",
                )}
              >
                <input
                  type="radio"
                  name={`${id}-interval`}
                  value={v}
                  checked={checked}
                  onChange={() => onChange({ intervalMinutes: v })}
                  className="sr-only"
                />
                {v} min
              </label>
            );
          })}
        </div>
        <FieldError id={`${id}-interval-error`} message={errors.intervalMinutes} />
      </fieldset>

      <div className="space-y-1.5">
        <label htmlFor={`${id}-idle`} className="text-sm font-medium">
          Idle detection
        </label>
        <Select
          id={`${id}-idle`}
          value={String(draft.idleThresholdMinutes)}
          onChange={(e) => onChange({ idleThresholdMinutes: Number(e.target.value) })}
          aria-describedby={`${id}-idle-hint`}
        >
          {idles.map((v) => (
            <option key={v} value={v}>
              {idleChoiceLabel(v)}
            </option>
          ))}
        </Select>
        <p id={`${id}-idle-hint`} className="text-xs text-muted-foreground">
          After this long without keyboard or mouse input the block ends; when the member is back they choose whether to keep the
          away time as work.
        </p>
        <FieldError id={`${id}-idle-error`} message={errors.idleThresholdMinutes} />
      </div>

      <div className="space-y-3 rounded-xl border border-border p-3">
        <div className="text-sm font-medium">What the agent captures</div>
        {permissionRows.map((p) => (
          <div key={p.key} className="space-y-3">
            <Switch checked={draft[p.key]} onChange={(v) => onChange({ [p.key]: v })} label={p.label} description={p.desc} />
            {p.key === "allowScreenshot" && (
              <div className="ml-4 border-l border-border pl-3">
                <Switch
                  checked={draft.blurScreenshots}
                  onChange={(v) => onChange({ blurScreenshots: v })}
                  label="Blur screenshots"
                  description={
                    draft.allowScreenshot
                      ? "Agents blur each capture before upload, so text can't be read"
                      : "Applies when screenshots are on"
                  }
                />
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
