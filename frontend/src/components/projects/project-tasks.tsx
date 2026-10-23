"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { CheckCircle2, Circle, Loader2, Pencil, Plus } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Modal } from "@/components/ui/modal";
import { Input } from "@/components/ui/input";
import { toast } from "@/components/toast";
import { requestErrorText } from "@/components/layout/request-error";
import { currentWorkspaceId } from "@/lib/live-session";
import {
  PROJECT_TASK_LIMITS,
  toPagedResult,
  type GetProjectTasksQuery,
  type ProjectTaskDto,
} from "@/lib/api-contract";
import {
  createProjectTaskApi,
  getProjectTasksApi,
  updateProjectTaskApi,
} from "@/hooks/useApi";

export type TaskFilter = "all" | "open" | "completed";

export interface ProjectTasksSectionProps {
  projectId: string;
  projectArchived: boolean;
  isLive: boolean;
  canMutate: boolean;
}

const PAGE_SIZE = 10;

export function ProjectTasksSection({
  projectId,
  projectArchived,
  isLive,
  canMutate,
}: ProjectTasksSectionProps) {
  const [tasks, setTasks] = useState<ProjectTaskDto[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [page, setPage] = useState(0);
  const [filter, setFilter] = useState<TaskFilter>("all");
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [refreshWarning, setRefreshWarning] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  // In-flight mutation trackers
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ProjectTaskDto | null>(null);

  // Scope tracking for discard of stale in-flight results
  const activeScopeRef = useRef<string>("");
  const currentWorkspace = isLive ? currentWorkspaceId() : null;

  const loadTasks = useCallback(async () => {
    if (!isLive) {
      setTasks([]);
      setTotalCount(0);
      setLoading(false);
      return;
    }

    const currentScope = `${currentWorkspace ?? ""}:${projectId}:${filter}:${page}`;
    activeScopeRef.current = currentScope;

    setLoading(true);
    setFetchError(null);

    const query: GetProjectTasksQuery = {
      projectId,
      skipCount: page * PAGE_SIZE,
      maxResultCount: PAGE_SIZE,
      isCompleted: filter === "open" ? false : filter === "completed" ? true : undefined,
    };

    try {
      const res = await getProjectTasksApi(query);
      if (activeScopeRef.current !== currentScope) return; // Discard stale in-flight response
      const paged = toPagedResult<ProjectTaskDto>(res);
      setTasks(paged.items);
      setTotalCount(paged.totalCount);
      setRefreshWarning(null);
    } catch (err) {
      if (activeScopeRef.current !== currentScope) return;
      setTasks([]);
      setTotalCount(0);
      setFetchError(requestErrorText(err, "Couldn't load tasks for this project."));
    } finally {
      if (activeScopeRef.current === currentScope) {
        setLoading(false);
      }
    }
  }, [isLive, currentWorkspace, projectId, filter, page]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadTasks();
  }, [loadTasks]);

  // Explicit retry sending ONLY GET
  async function reloadTasks() {
    if (refreshing || loading) return;
    setRefreshing(true);
    const query: GetProjectTasksQuery = {
      projectId,
      skipCount: page * PAGE_SIZE,
      maxResultCount: PAGE_SIZE,
      isCompleted: filter === "open" ? false : filter === "completed" ? true : undefined,
    };
    try {
      const res = await getProjectTasksApi(query);
      const paged = toPagedResult<ProjectTaskDto>(res);
      setTasks(paged.items);
      setTotalCount(paged.totalCount);
      setRefreshWarning(null);
    } catch {
      setRefreshWarning("Couldn't refresh tasks. Check your connection and try again.");
    } finally {
      setRefreshing(false);
    }
  }

  // Toggle completion: preserves existing title and description
  async function handleToggleComplete(task: ProjectTaskDto) {
    if (!canMutate || projectArchived || busyTaskId) return;
    setBusyTaskId(task.id);
    setRefreshWarning(null);

    const nextCompleted = !task.isCompleted;
    try {
      const updated = await updateProjectTaskApi(task.id, {
        title: task.title,
        description: task.description ?? null,
        isCompleted: nextCompleted,
      });

      // Reconcile locally immediately
      setTasks((prev) => prev.map((t) => (t.id === updated.id ? updated : t)));
      toast({
        title: updated.isCompleted ? "Task completed" : "Task reopened",
        tone: "success",
      });

      // Background list refresh
      try {
        const query: GetProjectTasksQuery = {
          projectId,
          skipCount: page * PAGE_SIZE,
          maxResultCount: PAGE_SIZE,
          isCompleted: filter === "open" ? false : filter === "completed" ? true : undefined,
        };
        const res = await getProjectTasksApi(query);
        const paged = toPagedResult<ProjectTaskDto>(res);
        setTasks(paged.items);
        setTotalCount(paged.totalCount);
      } catch {
        setRefreshWarning("Task was updated, but we couldn't refresh the list. Retry to update.");
      }
    } catch (err) {
      toast({
        tone: "danger",
        title: "Couldn't update task",
        description: requestErrorText(err, "The change wasn't saved."),
      });
    } finally {
      setBusyTaskId(null);
    }
  }

  const totalPages = Math.max(1, Math.ceil(totalCount / PAGE_SIZE));

  return (
    <Card>
      <CardHeader className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <CardTitle>Tasks</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {isLive ? `${totalCount} ${totalCount === 1 ? "task" : "tasks"}` : "Manage tasks for this project"}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Filter tabs */}
          {isLive && (
            <div className="flex rounded-lg border border-border bg-muted/40 p-0.5 text-xs" role="tablist" aria-label="Filter tasks">
              {(["all", "open", "completed"] as const).map((tab) => {
                const active = filter === tab;
                const label = tab === "all" ? "All" : tab === "open" ? "Open" : "Completed";
                return (
                  <button
                    key={tab}
                    type="button"
                    role="tab"
                    aria-selected={active}
                    onClick={() => {
                      if (filter !== tab) {
                        setFilter(tab);
                        setPage(0);
                      }
                    }}
                    className={`rounded-md px-2.5 py-1 font-medium transition-colors ${
                      active
                        ? "bg-card text-foreground shadow-xs"
                        : "text-muted-foreground hover:text-foreground"
                    }`}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          )}

          {canMutate && !projectArchived && (
            <Button size="sm" onClick={() => setCreateModalOpen(true)}>
              <Plus className="h-3.5 w-3.5" /> Add task
            </Button>
          )}
        </div>
      </CardHeader>

      <CardContent className="space-y-3">
        {/* Refresh failure alert banner */}
        {refreshWarning && (
          <div
            role="alert"
            className="flex items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning-on-tint"
          >
            <span className="min-w-0 flex-1 font-medium">{refreshWarning}</span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={refreshing || !!busyTaskId}
              onClick={() => void reloadTasks()}
              className="shrink-0"
            >
              {refreshing && (
                <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
              )}
              {refreshing ? "Retrying…" : "Retry"}
            </Button>
          </div>
        )}

        {/* Demo mode network-free empty state */}
        {!isLive && (
          <p className="py-6 text-center text-sm text-muted-foreground">
            No tasks in demo mode. Tasks are managed in live workspaces.
          </p>
        )}

        {/* Live mode loading */}
        {isLive && loading && tasks.length === 0 && (
          <p role="status" className="py-6 text-center text-sm text-muted-foreground">
            Loading tasks…
          </p>
        )}

        {/* Live mode fetch error */}
        {isLive && !loading && fetchError && (
          <div className="flex flex-col items-center gap-2 py-6 text-center">
            <p role="alert" className="text-sm text-muted-foreground">
              {fetchError}
            </p>
            <Button size="sm" variant="outline" onClick={() => void loadTasks()}>
              Try again
            </Button>
          </div>
        )}

        {/* Live mode empty state */}
        {isLive && !loading && !fetchError && tasks.length === 0 && (
          <p className="py-6 text-center text-sm text-muted-foreground">
            {filter === "open"
              ? "No open tasks."
              : filter === "completed"
                ? "No completed tasks."
                : "No tasks yet."}
          </p>
        )}

        {/* Task list */}
        {isLive && tasks.length > 0 && (
          <ul className="space-y-2" aria-label="Project tasks">
            {tasks.map((task) => {
              const isBusy = busyTaskId === task.id;
              const completed = task.isCompleted;
              return (
                <li
                  key={task.id}
                  className="flex items-start justify-between gap-3 rounded-xl border border-border/70 bg-card p-3 transition-colors hover:border-border"
                >
                  <div className="flex min-w-0 flex-1 items-start gap-3">
                    {/* Completion toggle / status */}
                    <button
                      type="button"
                      disabled={!canMutate || projectArchived || isBusy}
                      onClick={() => void handleToggleComplete(task)}
                      aria-label={
                        completed
                          ? `Mark "${task.title}" as open`
                          : `Mark "${task.title}" as completed`
                      }
                      className={`mt-0.5 shrink-0 rounded-full transition-colors ${
                        !canMutate || projectArchived
                          ? "cursor-default opacity-60"
                          : "hover:opacity-80"
                      }`}
                    >
                      {isBusy ? (
                        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
                      ) : completed ? (
                        <CheckCircle2 className="h-5 w-5 text-success" aria-hidden="true" />
                      ) : (
                        <Circle className="h-5 w-5 text-muted-foreground" aria-hidden="true" />
                      )}
                    </button>

                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span
                          className={`text-sm font-medium ${
                            completed ? "line-through text-muted-foreground" : "text-foreground"
                          }`}
                        >
                          {task.title}
                        </span>
                        {completed && <Badge tone="success">Completed</Badge>}
                      </div>
                      {task.description && (
                        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                          {task.description}
                        </p>
                      )}
                    </div>
                  </div>

                  {/* Actions */}
                  {canMutate && !projectArchived && (
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-8 w-8 text-muted-foreground hover:text-foreground"
                      onClick={() => setEditingTask(task)}
                      aria-label={`Edit task "${task.title}"`}
                      title="Edit task"
                    >
                      <Pencil className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}

        {/* Pagination controls */}
        {isLive && totalCount > PAGE_SIZE && (
          <div className="flex items-center justify-between border-t border-border/70 pt-3 text-xs text-muted-foreground">
            <span>
              Page {page + 1} of {totalPages}
            </span>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page === 0 || loading || refreshing}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
              >
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={(page + 1) * PAGE_SIZE >= totalCount || loading || refreshing}
                onClick={() => setPage((p) => p + 1)}
              >
                Next
              </Button>
            </div>
          </div>
        )}
      </CardContent>

      {/* Create Task Modal */}
      {createModalOpen && (
        <TaskFormModal
          title="New task"
          initialTitle=""
          initialDescription=""
          onClose={() => setCreateModalOpen(false)}
          onSubmit={async (newTitle, newDesc) => {
            const created = await createProjectTaskApi({
              projectId,
              title: newTitle,
              description: newDesc || null,
              isCompleted: false,
            });

            // Reconcile locally immediately
            if (page === 0 && filter !== "completed") {
              setTasks((prev) => [created, ...prev]);
            }
            setTotalCount((c) => c + 1);
            toast({ title: "Task created", tone: "success" });

            // Background reload
            try {
              const query: GetProjectTasksQuery = {
                projectId,
                skipCount: page * PAGE_SIZE,
                maxResultCount: PAGE_SIZE,
                isCompleted: filter === "open" ? false : filter === "completed" ? true : undefined,
              };
              const res = await getProjectTasksApi(query);
              const paged = toPagedResult<ProjectTaskDto>(res);
              setTasks(paged.items);
              setTotalCount(paged.totalCount);
            } catch {
              setRefreshWarning("Task was created, but we couldn't refresh the list. Retry to update.");
            }
          }}
        />
      )}

      {/* Edit Task Modal */}
      {editingTask && (
        <TaskFormModal
          title="Edit task"
          initialTitle={editingTask.title}
          initialDescription={editingTask.description ?? ""}
          onClose={() => setEditingTask(null)}
          onSubmit={async (updatedTitle, updatedDesc) => {
            const updated = await updateProjectTaskApi(editingTask.id, {
              title: updatedTitle,
              description: updatedDesc || null,
              isCompleted: editingTask.isCompleted,
            });

            // Reconcile locally immediately
            setTasks((prev) => prev.map((t) => (t.id === updated.id ? updated : t)));
            toast({ title: "Task updated", tone: "success" });

            // Background reload
            try {
              const query: GetProjectTasksQuery = {
                projectId,
                skipCount: page * PAGE_SIZE,
                maxResultCount: PAGE_SIZE,
                isCompleted: filter === "open" ? false : filter === "completed" ? true : undefined,
              };
              const res = await getProjectTasksApi(query);
              const paged = toPagedResult<ProjectTaskDto>(res);
              setTasks(paged.items);
              setTotalCount(paged.totalCount);
            } catch {
              setRefreshWarning("Task was updated, but we couldn't refresh the list. Retry to update.");
            }
          }}
        />
      )}
    </Card>
  );
}

interface TaskFormModalProps {
  title: string;
  initialTitle: string;
  initialDescription: string;
  onClose: () => void;
  onSubmit: (title: string, description: string) => Promise<void>;
}

function TaskFormModal({
  title,
  initialTitle,
  initialDescription,
  onClose,
  onSubmit,
}: TaskFormModalProps) {
  const [taskTitle, setTaskTitle] = useState(initialTitle);
  const [taskDescription, setTaskDescription] = useState(initialDescription);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmedTitle = taskTitle.trim();
    if (!trimmedTitle) {
      setError("The task needs a title.");
      return;
    }
    if (trimmedTitle.length > PROJECT_TASK_LIMITS.titleMaxLength) {
      setError(`The title cannot exceed ${PROJECT_TASK_LIMITS.titleMaxLength} characters.`);
      return;
    }
    if (taskDescription.length > PROJECT_TASK_LIMITS.descriptionMaxLength) {
      setError(`The description cannot exceed ${PROJECT_TASK_LIMITS.descriptionMaxLength} characters.`);
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(trimmedTitle, taskDescription.trim());
      onClose();
    } catch (err) {
      setError(requestErrorText(err, "Couldn't save task."));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Modal open onClose={onClose} title={title}>
      <form onSubmit={handleSubmit} className="space-y-4">
        {error && (
          <p role="alert" className="text-sm font-medium text-danger-on-tint">
            {error}
          </p>
        )}

        <div className="space-y-1.5">
          <label htmlFor="task-title-input" className="text-xs font-semibold text-foreground">
            Title
          </label>
          <Input
            id="task-title-input"
            value={taskTitle}
            onChange={(e) => setTaskTitle(e.target.value)}
            maxLength={PROJECT_TASK_LIMITS.titleMaxLength}
            placeholder="e.g. Redesign navigation"
            disabled={submitting}
            autoFocus
          />
        </div>

        <div className="space-y-1.5">
          <label htmlFor="task-desc-input" className="text-xs font-semibold text-foreground">
            Description <span className="font-normal text-muted-foreground">(optional)</span>
          </label>
          <textarea
            id="task-desc-input"
            value={taskDescription}
            onChange={(e) => setTaskDescription(e.target.value)}
            maxLength={PROJECT_TASK_LIMITS.descriptionMaxLength}
            rows={3}
            placeholder="Add details, criteria or context"
            disabled={submitting}
            className="w-full rounded-xl border border-input bg-card p-3 text-sm focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>

        <div className="flex justify-end gap-2 pt-2">
          <Button type="button" variant="outline" onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting && (
              <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            )}
            {submitting ? "Saving…" : "Save task"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
