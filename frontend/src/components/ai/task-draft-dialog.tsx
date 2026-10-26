"use client";

import { useEffect, useId, useRef, useState } from "react";
import { AlertTriangle, Loader2, Sparkles, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { requestErrorText } from "@/components/layout/request-error";
import { toast } from "@/components/toast";
import { AiDisclosurePopover } from "@/components/ai/ai-disclosure-popover";
import { AiFooter } from "@/components/ai/ai-footer";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { useAiRequest } from "@/hooks/use-ai-request";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import { createProjectTaskApi } from "@/hooks/useApi";
import type { AiTaskDraftsDto } from "@/lib/ai-contract";
import {
  AI_ANSWER_LABEL,
  AI_COPY,
  AI_GOAL_MAX_LENGTH,
  AI_GOAL_MIN_LENGTH,
  AI_MAX_TASK_DRAFTS,
  DEMO_NOTE,
  selectedTasksText,
  taskDraftProblem,
} from "@/lib/ai-assist";
import { PROJECT_TASK_LIMITS, type ProjectTaskDto } from "@/lib/api-contract";
import { cn } from "@/lib/utils";

/** One draft as the person is editing it. */
interface DraftRow {
  key: string;
  checked: boolean;
  title: string;
  description: string;
  rationale: string;
  duplicateOf: string | null;
  /** Why this one couldn't be saved (checked again on the next Add). */
  error: string | null;
}

const toRows = (result: AiTaskDraftsDto): DraftRow[] =>
  result.drafts.map((d, i) => ({
    key: `${result.meta.resultId}-${i}`,
    // Likely duplicates start unticked: nothing is added without a look.
    checked: !d.possibleDuplicateOfTaskId,
    title: d.title,
    description: d.description ?? "",
    rationale: d.rationale,
    duplicateOf: d.possibleDuplicateOfTaskId,
    error: null,
  }));

const fieldClass =
  "block w-full resize-y rounded-xl border border-input bg-card px-3 py-2 text-sm placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-60";

/**
 * "Draft tasks with AI" (task-drafts, AI assist spec §3.1, §18.4): describe a
 * goal, get up to eight suggested tasks, edit and tick the ones to keep, then
 * "Add N tasks" creates them one by one through the project's ordinary task
 * endpoint (POST /api/app/project-task), which checks the person's rights
 * again. Nothing is created until that click; a draft that couldn't be
 * created stays in the list with the reason, the rest are added.
 *
 * A demo drafts from the demo data and adds nothing (tasks live only in live
 * workspaces). Loaded on demand by components/projects/project-tasks.
 */
export function TaskDraftDialog({
  projectId,
  existingTasks,
  onClose,
  onCreated,
}: {
  projectId: string;
  /** The tasks the list shows, to name the one a draft may duplicate. */
  existingTasks: readonly ProjectTaskDto[];
  onClose: () => void;
  /** Some tasks were created (the list should reload). */
  onCreated: (count: number) => void;
}) {
  const { live, workspaceId, cache, userId } = useAiAssistant();
  const timeZone = useBrowserTimeZone();
  const ids = useId();
  const [goal, setGoal] = useState("");
  const [maxTasks, setMaxTasks] = useState(5);
  const [rows, setRows] = useState<DraftRow[] | null>(null);
  const [shown, setShown] = useState<AiTaskDraftsDto | null>(null);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const request = useAiRequest(
    "task-drafts",
    { projectId, goal: goal.trim(), maxTasks },
    { live, workspaceId, cache, demoContext: { userId, timeZone }, timeZone },
  );
  const { state } = request;
  const loading = state.phase === "loading";

  // A new result replaces the list (edits to the goal afterwards keep the drafts already shown).
  if (state.phase === "ready" && state.result !== shown) {
    setShown(state.result);
    setRows(toRows(state.result));
    setAddError(null);
  }

  const selected = rows?.filter((r) => r.checked) ?? [];
  const canDraft = goal.trim().length >= AI_GOAL_MIN_LENGTH && !loading && !adding;
  const update = (key: string, patch: Partial<DraftRow>) => setRows((list) => list?.map((r) => (r.key === key ? { ...r, ...patch } : r)) ?? null);
  const duplicateTitle = (id: string | null) => (id ? existingTasks.find((t) => t.id === id)?.title ?? null : null);

  async function add() {
    if (!rows || selected.length === 0 || adding) return;
    // Every ticked draft must be a valid task before anything is sent.
    const problems = new Map(selected.map((r) => [r.key, taskDraftProblem(r.title, r.description)]));
    if ([...problems.values()].some(Boolean)) {
      setRows(rows.map((r) => (problems.get(r.key) ? { ...r, error: problems.get(r.key) ?? null } : r)));
      setAddError("Fix the highlighted tasks first.");
      return;
    }
    setAddError(null);
    if (!live) {
      toast({ title: `${selected.length} ${selected.length === 1 ? "task" : "tasks"} drafted`, description: "Demo workspace — tasks are only saved in live workspaces." });
      onClose();
      return;
    }

    setAdding(true);
    const failed = new Map<string, string>();
    let created = 0;
    // One at a time, in the order shown: each goes through the ordinary task endpoint.
    for (const row of selected) {
      try {
        await createProjectTaskApi({ projectId, title: row.title.trim(), description: row.description.trim() || null, isCompleted: false });
        created++;
      } catch (err) {
        failed.set(row.key, requestErrorText(err, "Couldn't create this task."));
      }
    }
    if (!mounted.current) {
      if (created > 0) onCreated(created);
      return;
    }
    setAdding(false);
    if (created > 0) onCreated(created);
    if (failed.size === 0) {
      toast({ tone: "success", title: `${created} ${created === 1 ? "task" : "tasks"} added` });
      onClose();
      return;
    }
    // Keep only what still needs doing: the failed drafts (ticked, with their reason) and the unticked ones.
    setRows((list) =>
      (list ?? [])
        .filter((r) => !r.checked || failed.has(r.key))
        .map((r) => ({ ...r, error: failed.get(r.key) ?? null })),
    );
    setAddError(
      created > 0
        ? `Added ${created} of ${selected.length} tasks. ${failed.size} couldn't be created: the reason is shown with each one.`
        : `None of the ${selected.length} tasks could be created: the reason is shown with each one.`,
    );
  }

  const result = state.phase === "ready" ? state.result : shown;
  const demo = result?.meta.status === "demo";
  const goalId = `${ids}goal`;
  const goalCountId = `${ids}goal-count`;
  const countId = `${ids}count`;
  const addLabel = `Add ${selected.length} ${selected.length === 1 ? "task" : "tasks"}`;

  return (
    <Modal open onClose={onClose} title="Draft tasks with AI" description="Describe what you want to get done; review the drafts before adding any." className="max-w-2xl">
      <div className="space-y-5">
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (canDraft) request.run();
          }}
        >
          <div className="space-y-1.5">
            <label htmlFor={goalId} className="text-sm font-medium">
              Goal
            </label>
            <textarea
              id={goalId}
              value={goal}
              rows={3}
              maxLength={AI_GOAL_MAX_LENGTH}
              placeholder="e.g. Launch the new onboarding flow"
              aria-describedby={goalCountId}
              disabled={adding}
              onChange={(e) => setGoal(e.target.value)}
              className={fieldClass}
            />
            <p id={goalCountId} className="text-[11px] text-muted-foreground">
              {goal.length} / {AI_GOAL_MAX_LENGTH}
              {goal.trim().length < AI_GOAL_MIN_LENGTH && ` · at least ${AI_GOAL_MIN_LENGTH} characters`}
            </p>
          </div>
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div className="space-y-1.5">
              <label htmlFor={`${ids}max`} className="text-sm font-medium">
                How many tasks
              </label>
              <Select id={`${ids}max`} value={String(maxTasks)} onChange={(e) => setMaxTasks(Number(e.target.value))} disabled={adding} className="w-24">
                {Array.from({ length: AI_MAX_TASK_DRAFTS }, (_, i) => i + 1).map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </Select>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <AiDisclosurePopover />
              {loading ? (
                <Button type="button" variant="outline" size="sm" onClick={request.cancel}>
                  <X aria-hidden className="h-3.5 w-3.5" /> {AI_COPY.cancel}
                </Button>
              ) : (
                <Button type="submit" size="sm" disabled={!canDraft}>
                  <Sparkles aria-hidden className="h-3.5 w-3.5" /> {rows ? "Draft again" : "Draft tasks"}
                </Button>
              )}
            </div>
          </div>
        </form>

        <p role="status" aria-live="polite" className={loading ? "flex items-center gap-2 text-sm text-muted-foreground" : "sr-only"}>
          {loading && <Loader2 aria-hidden className="h-4 w-4 motion-safe:animate-spin" />}
          {loading ? "Drafting tasks..." : rows ? `${rows.length} ${rows.length === 1 ? "draft" : "drafts"} ready` : ""}
        </p>
        {state.phase === "error" && (
          <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-xl bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
            <span>{state.error.message}</span>
            {state.error.retryable && (
              <Button type="button" size="sm" variant="outline" onClick={() => request.run()}>
                {AI_COPY.retry}
              </Button>
            )}
          </div>
        )}

        {rows && rows.length === 0 && <p className="text-sm text-muted-foreground">No drafts came back for this goal. Try describing it differently.</p>}
        {rows && rows.length > 0 && result && (
          <div className="space-y-3">
            <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <Badge tone={demo ? "muted" : "info"}>{demo ? AI_COPY.sampleBadge : AI_COPY.aiBadge}</Badge>
              {demo ? DEMO_NOTE : `${AI_ANSWER_LABEL}. Edit anything before adding it.`}
            </p>
            <fieldset className="space-y-3" disabled={adding}>
              <legend className="mb-2 text-sm font-semibold">Suggested tasks</legend>
              {rows.map((row, i) => {
                const n = i + 1;
                const titleId = `${ids}t${n}`;
                const descId = `${ids}d${n}`;
                const errorId = `${ids}e${n}`;
                const similar = duplicateTitle(row.duplicateOf);
                return (
                  <div key={row.key} className={cn("space-y-2 rounded-xl border p-3", row.error ? "border-danger" : "border-border", !row.checked && "bg-muted/30")}>
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <label className="flex items-center gap-2 text-sm font-medium">
                        <input
                          type="checkbox"
                          checked={row.checked}
                          onChange={(e) => update(row.key, { checked: e.target.checked, error: null })}
                          className="h-4 w-4 shrink-0 accent-[var(--primary)]"
                        />
                        Add task {n}
                      </label>
                      {row.duplicateOf && (
                        <Badge tone="warning" className="gap-1">
                          <AlertTriangle aria-hidden className="h-3 w-3" />
                          {similar ? `Similar to an existing task: ${similar}` : "Similar to an existing task"}
                        </Badge>
                      )}
                    </div>
                    <div className="space-y-1">
                      <label htmlFor={titleId} className="text-xs font-semibold text-foreground">
                        Task {n} title
                      </label>
                      <Input
                        id={titleId}
                        value={row.title}
                        maxLength={PROJECT_TASK_LIMITS.titleMaxLength}
                        aria-invalid={row.error ? true : undefined}
                        aria-describedby={row.error ? errorId : undefined}
                        onChange={(e) => update(row.key, { title: e.target.value, error: null })}
                      />
                    </div>
                    <div className="space-y-1">
                      <label htmlFor={descId} className="text-xs font-semibold text-foreground">
                        Task {n} description <span className="font-normal text-muted-foreground">(optional)</span>
                      </label>
                      <textarea
                        id={descId}
                        value={row.description}
                        rows={2}
                        maxLength={PROJECT_TASK_LIMITS.descriptionMaxLength}
                        onChange={(e) => update(row.key, { description: e.target.value, error: null })}
                        className={fieldClass}
                      />
                    </div>
                    {row.rationale && <p className="text-xs text-muted-foreground">Why: {row.rationale}</p>}
                    {row.error && (
                      <p id={errorId} className="text-xs font-medium text-danger-on-tint">
                        {row.error}
                      </p>
                    )}
                  </div>
                );
              })}
            </fieldset>
            <AiFooter meta={result.meta} copyText={rows.map((r) => `- ${r.title}`).join("\n")} compact />
          </div>
        )}

        {addError && (
          <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
            {addError}
          </p>
        )}
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-4">
          <p id={countId} role="status" aria-live="polite" className="text-xs text-muted-foreground">
            {rows && rows.length > 0 ? selectedTasksText(selected.length) : ""}
          </p>
          <div className="flex gap-2">
            <Button type="button" variant="ghost" onClick={onClose} disabled={adding}>
              {AI_COPY.cancel}
            </Button>
            <Button type="button" onClick={() => void add()} disabled={!rows || selected.length === 0 || adding} aria-describedby={countId}>
              {adding && <Loader2 aria-hidden className="h-3.5 w-3.5 motion-safe:animate-spin" />}
              {adding ? "Adding…" : addLabel}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
