"use client";

import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Info, Loader2, Play, SearchX } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SegmentedControl } from "@/components/ui/toolbar";
import { ActivityDrawer, type ProjectLabel } from "@/components/activities/activity-drawer";
import { ActivityFilterBar } from "@/components/activities/activity-filter-bar";
import type { NoteEditing } from "@/components/activities/note-editor";
import { ScreenTile, SessionCard, isViewable, type CardMember } from "@/components/activities/session-card";
import { ScreenshotViewer, type ViewerItem } from "@/components/activities/screenshot-viewer";
import { useTeamPresence, useVisibleInterval } from "@/components/monitor/use-team-presence";
import { useSession } from "@/components/session-provider";
import { activities as demoActivities, projectById, userById, users } from "@/lib/tenant-data";
import { ApiError, getPagedApi, updateActivityNote } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { agoLabel, referenceNow } from "@/lib/live-session";
import {
  DEFAULT_SESSION_DESCRIPTION,
  demoSessionDetails,
  formatBlockLength,
  groupSessionsByDay,
  readSessionDetails,
  sessionDetailsById,
  type SessionDay,
  type SessionDetails,
} from "@/lib/activity-report";
import { mapApiActivities } from "@/lib/live-dataset";
import {
  activityApps,
  activityDateRange,
  applyActivityFilters,
  countActiveFilters,
  defaultActivityFilters,
  formatActivityRange,
  hasLocalFilters,
  type ActivityFilters,
} from "@/lib/activity-filters";
import { activitiesRoute, canEditActivityNote, normalizeActivityNote, withPaging } from "@/lib/api-contract";
import {
  demoPresence,
  latestBlockByUser,
  liveBlockIds,
  mergePresence,
  presenceUserStatus,
  uploadPresence,
  type MemberPresence,
} from "@/lib/monitor-live";
import { reportAccessFor } from "@/lib/report-math";
import { timeZoneLabel } from "@/lib/time-zone";
import { scopeActivities } from "@/lib/scope";
import type { Activity } from "@/lib/types";

type ViewMode = "sessions" | "screens";

/** Cards rendered per page on the activity list (see visibleCount below). */
const PAGE_SIZE = 24;

/**
 * Live rows per request. The server filters by the selected days, member and
 * project and pages newest first; "Load older sessions" asks for the next page,
 * and the page always says how many of the range's sessions are loaded.
 */
const LIVE_PAGE_SIZE = 500;

/** How often "who is tracking now" is refreshed while the tab is visible. */
const PRESENCE_REFRESH_MS = 60_000;

/**
 * A live request's To is pinned to the moment it was made (plus a little room
 * for a device clock running ahead), so uploads landing while older pages are
 * read can't shift them.
 */
const CLOCK_ROOM_MS = 10 * 60_000;

/** One live list: the rows of a server query, page by page. */
interface LiveList {
  /** The query it answers (dates, zone, member, project, reload). */
  key: string;
  /** The request without paging, for the next page. */
  endpoint: string;
  /** Raw ActivityDto rows, newest first, each once. */
  rows: unknown[];
  totalCount: number;
  /** The clock the date presets were read from. */
  now: number;
}

function rowId(row: unknown): unknown {
  return row && typeof row === "object" ? (row as { id?: unknown }).id : undefined;
}

/** `rows` then the `more` that aren't already there (offset paging over a growing list can repeat a row). */
function appendRows(rows: unknown[], more: unknown[]): unknown[] {
  const seen = new Set(rows.map(rowId));
  return [...rows, ...more.filter((r) => !seen.has(rowId(r)))];
}

/** Why a note couldn't be saved, in words. */
function noteErrorMessage(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.validationErrors.length > 0) return err.validationErrors.map((v) => v.message).join(" ");
    if (err.serverMessage) return err.serverMessage;
    if (err.status === 403) return "You can't edit this note — only its owner (within 30 days) or a team manager can.";
    if (err.status === 404) return "This session no longer exists.";
    return `Couldn't save the note (HTTP ${err.status}). Try again.`;
  }
  return "Couldn't reach the server to save the note. Check your connection and try again.";
}

export default function ActivitiesPage() {
  return (
    <Suspense fallback={<div className="py-16 text-center text-sm text-muted-foreground">Loading activity…</div>}>
      <ActivitiesPageInner />
    </Suspense>
  );
}

function ActivitiesPageInner() {
  const { user, isLive, livePolicies } = useSession();
  // Team managers by the real grant (Tracker.Team.Manage) when live; by role in the demo.
  const canManageTeam = reportAccessFor(user.role, isLive ? livePolicies : null).manageTeam;
  const router = useRouter();
  const searchParams = useSearchParams();
  const [filters, setFilters] = useState<ActivityFilters>(defaultActivityFilters);
  const [selected, setSelected] = useState<Activity | null>(null);
  const [viewerIndex, setViewerIndex] = useState<number | null>(null);
  // Set when the viewer was opened from a session's report, so closing it goes back there.
  const [viewerReturnTo, setViewerReturnTo] = useState<Activity | null>(null);
  const [view, setView] = useState<ViewMode>(searchParams.get("view") === "screens" ? "screens" : "sessions");

  // Every day, date preset and clock time on the page reads in the reporting zone.
  const zone = useReportingZone(isLive);
  const timeZone = zone.timeZone;

  // The clock "live now" and the note rules read; ticks while the tab is visible.
  const [clock, setClock] = useState(() => referenceNow().getTime());
  useVisibleInterval(() => setClock(referenceNow().getTime()), PRESENCE_REFRESH_MS, true);

  // LIVE: the server filters by the selected days, member and project and pages
  // the rows (demo mode never touches the network). A query's answer is kept
  // under its key, so a change of filters shows "loading" rather than rows of
  // another range.
  const [reloadKey, setReloadKey] = useState(0);
  const [liveList, setLiveList] = useState<LiveList | null>(null);
  const [liveFailure, setLiveFailure] = useState<string | null>(null);
  const [more, setMore] = useState<{ key: string; loading: boolean; failed: boolean } | null>(null);
  // Only what the server filters by: typing a search doesn't ask again.
  const { rangeKey, customFrom, customTo, memberId, projectId } = filters;
  const serverFilters = useMemo(
    () => ({ rangeKey, customFrom, customTo, memberId, projectId }),
    [rangeKey, customFrom, customTo, memberId, projectId],
  );
  const queryKey = JSON.stringify([timeZone, serverFilters, reloadKey]);

  useEffect(() => {
    // Wait for the reporting zone, so the range isn't fetched once in the wrong zone and again in the right one.
    if (!isLive || !zone.ready) return;
    const controller = new AbortController();
    const now = Date.now();
    const range = activityDateRange(serverFilters, { now: new Date(now), timeZone });
    const endpoint = activitiesRoute({
      from: Number.isFinite(range.from) ? new Date(range.from) : null,
      to: new Date(Math.min(range.to, now + CLOCK_ROOM_MS)),
      userId: serverFilters.memberId,
      projectId: serverFilters.projectId,
    });
    getPagedApi<unknown>(withPaging(endpoint, 0, LIVE_PAGE_SIZE), { signal: controller.signal })
      .then((page) => {
        if (controller.signal.aborted) return;
        setLiveList({ key: queryKey, endpoint, rows: appendRows([], page.items), totalCount: page.totalCount, now });
        setLiveFailure(null);
      })
      .catch(() => {
        if (!controller.signal.aborted) setLiveFailure(queryKey);
      });
    return () => controller.abort();
  }, [isLive, zone.ready, queryKey, serverFilters, timeZone]);

  const loaded = isLive && liveList?.key === queryKey ? liveList : null;
  const serverMore = !!loaded && loaded.rows.length < loaded.totalCount;
  const loadingMore = !!loaded && more?.key === loaded.key && more.loading;

  async function loadOlder() {
    if (!loaded || !serverMore || loadingMore) return;
    const { key, endpoint, rows } = loaded;
    setMore({ key, loading: true, failed: false });
    try {
      const page = await getPagedApi<unknown>(withPaging(endpoint, rows.length, LIVE_PAGE_SIZE));
      setLiveList((prev) =>
        prev && prev.key === key
          ? { ...prev, rows: appendRows(prev.rows, page.items), totalCount: Math.max(prev.totalCount, page.totalCount) }
          : prev,
      );
      setMore(null);
      setVisibleCount((n) => n + PAGE_SIZE);
    } catch {
      setMore({ key, loading: false, failed: true });
    }
  }

  // Who is tracking right now (agents' heartbeats); the demo reads its seeded roster.
  const team = useTeamPresence(isLive, PRESENCE_REFRESH_MS);

  // Notes edited on this page, by block: they win over what the list was loaded with.
  const [notes, setNotes] = useState<ReadonlyMap<string, string | null>>(() => new Map());

  useEffect(() => {
    setView(searchParams.get("view") === "screens" ? "screens" : "sessions");
  }, [searchParams]);

  // Project names/colors: the session's dataset holds the tenant's REAL
  // projects in a live session and the seeded ones in demo mode.
  const resolveProject = useCallback((projectId: string): ProjectLabel | undefined => {
    const p = projectById(projectId);
    return p ? { title: p.title, color: p.color } : undefined;
  }, []);

  // Live activity, mapped with guarded JSON parsing (a malformed window list
  // from an agent must never crash the page).
  const liveActivities: Activity[] | null = useMemo(() => (loaded ? mapApiActivities(loaded.rows) : null), [loaded]);
  const liveDetails = useMemo(() => (loaded ? sessionDetailsById(loaded.rows) : null), [loaded]);

  // Live: real rows only. A failed load shows an error, never the demo rows.
  const list = useMemo(() => {
    const rows = isLive ? (liveActivities ?? []) : demoActivities;
    if (notes.size === 0) return rows;
    return rows.map((a) => (notes.has(a.id) ? { ...a, description: notes.get(a.id) ?? DEFAULT_SESSION_DESCRIPTION } : a));
  }, [isLive, liveActivities, notes]);
  const liveLoading = isLive && !loaded && liveFailure !== queryKey;
  const liveError = isLive && !loaded && liveFailure === queryKey;

  // What newer agents report per block (the demo's come from its own rows), with edited notes applied.
  const detailsOf = useCallback(
    (a: Activity): SessionDetails | undefined => {
      const base = isLive ? liveDetails?.get(a.id) : demoSessionDetails(a);
      if (!base || !notes.has(a.id)) return base;
      return { ...base, note: notes.get(a.id) ?? null };
    },
    [isLive, liveDetails, notes],
  );

  const intervalOf = useCallback((projectId: string) => projectById(projectId)?.intervalMinutes, []);
  const presence: Map<string, MemberPresence> = useMemo(() => {
    if (!isLive) return demoPresence(users, latestBlockByUser(demoActivities));
    if (team.devices) return mergePresence(team.devices, new Date(clock));
    // An older server without heartbeats: recent uploads stand in.
    if (team.unsupported) return uploadPresence(latestBlockByUser(list), new Date(clock), intervalOf);
    return new Map();
  }, [isLive, team.devices, team.unsupported, list, clock, intervalOf]);
  const liveIds = useMemo(() => liveBlockIds(list, presence, new Date(clock), intervalOf), [list, presence, clock, intervalOf]);
  const trackingNow = useMemo(() => [...presence.values()].filter((p) => p.state === "tracking").length, [presence]);

  const memberOf = useCallback(
    (a: Activity): CardMember => {
      const u = userById(a.userId);
      const state = presence.get(a.userId)?.state;
      return { name: u?.name ?? "Unknown member", status: state ? presenceUserStatus(state) : (u?.status ?? "offline") };
    },
    [presence],
  );

  // The owner of a block may edit its note for 30 days; team managers (Tracker.Team.Manage) always may.
  const noteEditing = useCallback(
    (a: Activity): NoteEditing => ({
      canEdit: canEditActivityNote({
        isOwner: a.userId === user.id,
        canManageTeam,
        startedAt: a.startedAt,
        now: new Date(clock),
      }),
      onSave: async (note) => {
        if (!isLive) {
          // The demo keeps the edit for this visit; nothing is sent anywhere.
          setNotes((prev) => new Map(prev).set(a.id, normalizeActivityNote(note)));
          return;
        }
        let saved: string | null;
        try {
          const dto = await updateActivityNote(a.id, note);
          saved = dto && typeof dto === "object" ? readSessionDetails(dto).note : normalizeActivityNote(note);
        } catch (err) {
          throw new Error(noteErrorMessage(err));
        }
        setNotes((prev) => new Map(prev).set(a.id, saved));
      },
    }),
    [clock, isLive, user.id, canManageTeam],
  );

  function setViewMode(mode: ViewMode) {
    setView(mode);
    router.replace(mode === "screens" ? "/activities?view=screens" : "/activities", { scroll: false });
  }

  const base = useMemo(() => scopeActivities(user, list), [user, list]);
  // The presets read the clock of the query that loaded the rows (live) or the demo's frozen one, in the reporting zone.
  const filterNow = useMemo(() => (loaded ? new Date(loaded.now) : referenceNow()), [loaded]);
  const filtered = useMemo(
    () =>
      applyActivityFilters(base, filters, {
        now: filterNow,
        timeZone,
        isLive: (a) => liveIds.has(a.id),
        isMeasured: (a) => detailsOf(a)?.measured ?? true,
      }),
    [base, filters, filterNow, timeZone, liveIds, detailsOf],
  );
  const range = useMemo(() => activityDateRange(filters, { now: filterNow, timeZone }), [filters, filterNow, timeZone]);
  const apps = useMemo(() => activityApps(base), [base]);
  const updatedAt = liveActivities?.[0]?.endedAt;
  const total = loaded && !hasLocalFilters(filters) ? Math.max(loaded.totalCount, filtered.length) : filtered.length;
  // Nothing at all in the default scope (the server already applied the dates, member and project):
  // point at the desktop agent rather than at the filters.
  const noTrackedTime = isLive && !!loaded && loaded.totalCount === 0 && countActiveFilters(filters) === 0;

  // The slideshow walks every screenshot in the current (filtered, sorted) list,
  // not just the cards rendered so far.
  const viewerItems: ViewerItem[] = useMemo(
    () =>
      filtered
        .filter((a) => isViewable(a, isLive))
        .map((a) => ({
          activity: a,
          member: memberOf(a),
          project: resolveProject(a.projectId),
          details: detailsOf(a),
          liveNow: liveIds.has(a.id),
          note: noteEditing(a),
        })),
    [filtered, isLive, memberOf, resolveProject, detailsOf, liveIds, noteEditing],
  );
  const viewerIndexOf = useMemo(() => new Map(viewerItems.map((it, n) => [it.activity.id, n])), [viewerItems]);
  const openViewer = (a: Activity, returnTo: Activity | null = null) => {
    const n = viewerIndexOf.get(a.id);
    if (n === undefined) return;
    setViewerReturnTo(returnTo);
    setViewerIndex(n);
  };
  const closeViewer = () => {
    setViewerIndex(null);
    if (viewerReturnTo) setSelected(viewerReturnTo);
    setViewerReturnTo(null);
  };

  // The Screens view is a screenshot gallery: blocks without one are listed under Sessions only.
  const shown = useMemo(
    () => (view === "screens" ? viewerItems.map((it) => it.activity) : filtered),
    [view, viewerItems, filtered],
  );
  const hiddenInScreens = view === "screens" ? filtered.length - shown.length : 0;

  // A busy team produces hundreds of sessions a day. Render a page at a time
  // rather than every card at once, and reset the window whenever the result
  // set changes so a new filter always starts at the top.
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [filters, view, timeZone]);
  const visible = useMemo(() => shown.slice(0, visibleCount), [shown, visibleCount]);
  const remaining = shown.length - visible.length;

  // Time-ordered lists read as days; other sorts (by activity) stay one flat grid.
  const chronological = filters.sort === "recent" || filters.sort === "oldest";
  const groups: SessionDay<Activity>[] = useMemo(
    () =>
      chronological
        ? groupSessionsByDay(visible, filterNow, {
            timeZone,
            isMeasured: (a) => detailsOf(a)?.measured ?? true,
            idleSecondsOf: (a) => detailsOf(a)?.idleSeconds,
          })
        : [{ key: "all", label: "", items: visible, seconds: 0, activity: null, idleSeconds: 0 }],
    [chronological, visible, filterNow, timeZone, detailsOf],
  );

  // The block the drawer shows, with any note edited since it was opened.
  const listById = useMemo(() => new Map(list.map((a) => [a.id, a])), [list]);
  const selectedActivity = selected ? (listById.get(selected.id) ?? selected) : null;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Monitor"
        title="Activity"
        description={
          <>
            {total} tracked {total === 1 ? "session" : "sessions"} · {formatActivityRange(range)}
            {updatedAt ? ` · updated ${agoLabel(updatedAt)}` : ""}
            {" · "}
            <span title={timeZone}>times in {timeZoneLabel(timeZone, clock)}</span>
            {!isLive && (
              <>
                {" · "}
                <span className="text-muted-foreground/80">previews are mock placeholders</span>
              </>
            )}
          </>
        }
        actions={
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setViewerIndex(0)}
              disabled={viewerItems.length === 0}
              title="Go through the screenshots one by one"
            >
              <Play className="h-3.5 w-3.5" /> Slideshow
            </Button>
            <SegmentedControl
              value={view}
              onChange={setViewMode}
              options={[
                { value: "sessions", label: "Sessions" },
                { value: "screens", label: "Screens" },
              ]}
            />
          </div>
        }
      />

      <ActivityFilterBar
        value={filters}
        onChange={setFilters}
        resultCount={filtered.length}
        totalCount={base.length}
        showMemberFilter={user.role === "owner" || user.role === "admin"}
        apps={apps}
        trackingNow={trackingNow}
        timeZone={timeZone}
      />

      {liveError && (
        <p role="alert" className="text-xs text-danger-on-tint">
          Couldn’t load activity from the server.{" "}
          <button
            type="button"
            onClick={() => setReloadKey((k) => k + 1)}
            className="font-medium underline underline-offset-2 hover:text-foreground"
          >
            Retry
          </button>
        </p>
      )}

      {serverMore && loaded && (
        <div
          role="status"
          className="flex flex-col gap-2 rounded-2xl border border-border bg-muted/40 px-4 py-3 text-sm sm:flex-row sm:items-center sm:justify-between"
        >
          <p className="flex items-start gap-2 text-muted-foreground">
            <Info aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <span>
              Loaded the {loaded.rows.length.toLocaleString("en")} most recent of {loaded.totalCount.toLocaleString("en")}{" "}
              sessions in this range.
              {hasLocalFilters(filters) ? " Search and the filters only look at the loaded ones." : ""}
            </span>
          </p>
          <Button variant="outline" size="sm" onClick={() => void loadOlder()} disabled={loadingMore} className="shrink-0">
            {loadingMore && <Loader2 aria-hidden className="h-3.5 w-3.5 motion-safe:animate-spin" />}
            Load older sessions
          </Button>
        </div>
      )}
      {more?.failed && loaded && more.key === loaded.key && (
        <p role="alert" className="text-xs text-danger-on-tint">
          Couldn’t load older sessions. Try again.
        </p>
      )}

      {noTrackedTime && (
        <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <SearchX className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No activity in the last 7 days</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Sessions appear here a few minutes after the desktop agent uploads its first block.
            </p>
          </div>
          <Link href="/download" className="text-sm text-primary-strong hover:underline">
            Get the desktop agent
          </Link>
        </Card>
      )}

      {!liveLoading && !liveError && !noTrackedTime && filtered.length === 0 && (
        <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <SearchX className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No activities match your filters</p>
            <p className="mt-1 text-sm text-muted-foreground">Try widening the date range or clearing some filters.</p>
          </div>
          <button onClick={() => setFilters(defaultActivityFilters)} className="text-sm text-primary hover:underline">
            Reset filters
          </button>
        </Card>
      )}

      {liveLoading ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading activity…</p>
      ) : (
        groups.map((day) => (
          <section key={day.key} aria-label={day.label || "Sessions"} className="space-y-3">
            {day.label && <DayHeader day={day} />}
            {view === "sessions" ? (
              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                {day.items.map((a) => (
                  <SessionCard
                    key={a.id}
                    activity={a}
                    member={memberOf(a)}
                    project={resolveProject(a.projectId)}
                    live={isLive}
                    details={detailsOf(a)}
                    liveNow={liveIds.has(a.id)}
                    timeZone={timeZone}
                    onView={viewerIndexOf.has(a.id) ? () => openViewer(a) : undefined}
                    onOpen={() => setSelected(a)}
                  />
                ))}
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 2xl:grid-cols-5">
                {day.items.map((a) => (
                  <ScreenTile
                    key={a.id}
                    activity={a}
                    member={memberOf(a)}
                    live={isLive}
                    details={detailsOf(a)}
                    liveNow={liveIds.has(a.id)}
                    timeZone={timeZone}
                    onView={() => openViewer(a)}
                  />
                ))}
              </div>
            )}
          </section>
        ))
      )}

      {hiddenInScreens > 0 && !liveLoading && (
        <p className="text-center text-xs text-muted-foreground">
          {hiddenInScreens} session{hiddenInScreens === 1 ? "" : "s"} without a screenshot{" "}
          {hiddenInScreens === 1 ? "is" : "are"} listed under Sessions only.
        </p>
      )}

      {remaining > 0 ? (
        <div className="flex flex-col items-center gap-2 pt-2">
          <p className="text-xs text-muted-foreground">
            Showing {visible.length} of {shown.length} {view === "screens" ? "screenshots" : "sessions"}
          </p>
          <Button variant="outline" onClick={() => setVisibleCount((n) => n + PAGE_SIZE)}>
            Show {Math.min(remaining, PAGE_SIZE)} more
          </Button>
        </div>
      ) : (
        serverMore &&
        loaded &&
        !liveLoading && (
          <div className="flex flex-col items-center gap-2 pt-2">
            <p className="text-xs text-muted-foreground">
              {(loaded.totalCount - loaded.rows.length).toLocaleString("en")} older{" "}
              {loaded.totalCount - loaded.rows.length === 1 ? "session" : "sessions"} in this range aren’t loaded yet
            </p>
            <Button variant="outline" onClick={() => void loadOlder()} disabled={loadingMore}>
              {loadingMore && <Loader2 aria-hidden className="h-4 w-4 motion-safe:animate-spin" />}
              Load older sessions
            </Button>
          </div>
        )
      )}

      {/* A fresh drawer per opened session: its captures are released on close, so
          reopening (e.g. back from the viewer) must never show the released images. */}
      <ActivityDrawer
        key={selected?.id ?? "closed"}
        activity={selectedActivity}
        project={selectedActivity ? resolveProject(selectedActivity.projectId) ?? null : null}
        details={selectedActivity ? detailsOf(selectedActivity) : undefined}
        note={selectedActivity ? noteEditing(selectedActivity) : undefined}
        liveNow={!!selectedActivity && liveIds.has(selectedActivity.id)}
        memberStatus={selectedActivity ? memberOf(selectedActivity).status : undefined}
        timeZone={timeZone}
        onClose={() => setSelected(null)}
        onViewScreenshot={
          selected && viewerIndexOf.has(selected.id)
            ? () => {
                // One dialog at a time: the report steps aside for the viewer and comes back after.
                openViewer(selected, selected);
                setSelected(null);
              }
            : undefined
        }
      />

      {/* Mounted only while open, so everything it downloaded is released on close. */}
      {viewerIndex !== null && (
        <ScreenshotViewer
          items={viewerItems}
          index={viewerIndex}
          live={isLive}
          timeZone={timeZone}
          onIndexChange={setViewerIndex}
          onClose={closeViewer}
          onOpenDetails={(a) => {
            setViewerIndex(null);
            setViewerReturnTo(null);
            setSelected(a);
          }}
        />
      )}
    </PageStack>
  );
}

/** A day's heading over its sessions: the date, and how the day went at a glance. */
function DayHeader({ day }: { day: SessionDay<Activity> }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b border-border pb-2">
      <h2 className="text-sm font-semibold">{day.label}</h2>
      <p className="text-xs text-muted-foreground">
        {day.items.length} session{day.items.length === 1 ? "" : "s"} · {formatBlockLength(day.seconds)} tracked
        {day.idleSeconds > 0 ? ` · ${formatBlockLength(day.idleSeconds)} idle` : ""} ·{" "}
        {day.activity === null ? "activity not measured" : `${day.activity}% avg activity`}
      </p>
    </div>
  );
}
