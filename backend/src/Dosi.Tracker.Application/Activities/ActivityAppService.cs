using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Authorization;
using Volo.Abp.BlobStoring;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Uow;
using Volo.Abp.Users;

using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Activities;

[Authorize]
public class ActivityAppService : TrackerAppService, IActivityAppService
{
    /// <summary>Largest decoded screen capture accepted (a JPEG at most 1920 px wide is typically 150-500 KB).
    /// Larger, invalid or disallowed captures are dropped (logged); the time block itself is always stored.</summary>
    public const int MaxScreenCaptureBytes = 8 * 1024 * 1024;

    /// <summary>Largest decoded thumbnail accepted (a JPEG at most 480 px wide).</summary>
    public const int MaxThumbCaptureBytes = 512 * 1024;

    /// <summary>Largest decoded webcam frame accepted.</summary>
    public const int MaxWebcamCaptureBytes = 2 * 1024 * 1024;

    /// <summary>Tolerated device-clock drift: a block may end at most this far in the server's future.</summary>
    public static readonly TimeSpan MaxFutureSkew = TimeSpan.FromMinutes(5);

    /// <summary>Longest block accepted. Fixed rather than derived from the project's interval (5..60 min), so a
    /// later interval change can never reject blocks recorded, or queued offline, under the previous one.</summary>
    public static readonly TimeSpan MaxBlockLength = TimeSpan.FromMinutes(120);

    /// <summary>Back-to-back blocks may overlap by up to this much (clock jitter); more is double-counting.</summary>
    public static readonly TimeSpan AllowedOverlap = TimeSpan.FromSeconds(60);

    /// <summary>A screen capture time reported by the agent is kept while it lies at most this far outside the block.</summary>
    public static readonly TimeSpan CaptureTimeTolerance = TimeSpan.FromSeconds(60);

    /// <summary>How long after its start the owner of a block may still change its note.</summary>
    public static readonly TimeSpan NoteEditWindow = TimeSpan.FromDays(30);

    /// <summary>How long an upload waits for another upload of the same member to finish.</summary>
    private static readonly TimeSpan IngestLockTimeout = TimeSpan.FromSeconds(30);

    private const int MaxAppNameLength = 256;
    private const int MaxWindowTitleLength = 512;

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IBlobContainer _blobContainer;
    private readonly ISubscriptionAccessChecker _subscriptionAccessChecker;
    private readonly IAbpDistributedLock _distributedLock;
    private readonly ActivityIngestOptions _ingestOptions;

    public ActivityAppService(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<DeviceRevocation, Guid> revocationRepository,
        IBlobContainer blobContainer,
        ISubscriptionAccessChecker subscriptionAccessChecker,
        IAbpDistributedLock distributedLock,
        IOptions<ActivityIngestOptions> ingestOptions)
    {
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _revocationRepository = revocationRepository;
        _blobContainer = blobContainer;
        _subscriptionAccessChecker = subscriptionAccessChecker;
        _distributedLock = distributedLock;
        _ingestOptions = ingestOptions.Value;
    }

    /// <summary>A capture decoded and checked, ready to be stored with its block.</summary>
    private sealed record PendingCapture(string Kind, byte[] Bytes, string ContentType, DateTime CapturedAt, bool Blurred);

    public async Task<ActivityDto> CreateAsync(CreateActivityDto input)
    {
        var userId = CurrentUser.GetId(); // Throws if not authenticated; activities must belong to a real user.

        if (input.ClientActivityId == Guid.Empty)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityClientIdRequired)
                .WithData("clientActivityId", input.ClientActivityId);
        }

        // One upload per member at a time, from the checks below until the block is committed: a retry racing
        // its original, or two devices uploading overlapping blocks, cannot both pass the duplicate and overlap
        // checks. The unique (WorkspaceId, UserId, ClientActivityId) index stays the backstop across server instances
        // when no distributed lock provider is configured.
        await using var ingestLock = await _distributedLock.TryAcquireAsync(IngestLockName(userId), IngestLockTimeout);
        if (ingestLock == null)
        {
            throw new AbpException(
                $"Another upload of this member is still being stored; retry the block {input.ClientActivityId} later.");
        }

        // Idempotent: an agent retrying an upload gets the stored block back, whatever changed since (membership,
        // archive, subscription). Scoped to the caller, so a guessed or colliding client id never discloses another
        // member's activity.
        var stored = await FindOwnBlockAsync(userId, input.ClientActivityId);
        if (stored != null)
        {
            return await MapToDtoWithCapturesAsync(stored);
        }

        // Deny subsequent activity ingestion for a revoked logical device id before persistence or blob side effects.
        if (input.DeviceId.HasValue && input.DeviceId.Value != Guid.Empty)
        {
            var isRevoked = await _revocationRepository.AnyAsync(r =>
                r.WorkspaceId == CurrentWorkspace.GetId() &&
                r.UserId == userId &&
                r.DeviceId == input.DeviceId.Value);

            if (isRevoked)
            {
                throw new BusinessException(TrackerDomainErrorCodes.DeviceRevoked)
                    .WithData("deviceId", input.DeviceId.Value);
            }
        }

        var project = await GetUploadProjectAsync(userId, input.ProjectId);

        // Client ids are unique per member; one already used by another member's block is refused without revealing it.
        if (await _activityRepository.AnyAsync(x => x.ClientActivityId == input.ClientActivityId && x.UserId != userId))
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityClientIdConflict)
                .WithData("clientActivityId", input.ClientActivityId);
        }

        await ValidateBlockAsync(userId, project, input);

        if (!await _subscriptionAccessChecker.IsUsableAsync())
        {
            throw new BusinessException(TrackerDomainErrorCodes.SubscriptionInactive);
        }

        var activity = BuildActivity(userId, project, input);
        var captures = DecodeCaptures(project, input);

        return await StoreAsync(activity, captures);
    }

    public async Task<PagedResultDto<ActivityDto>> GetListAsync(GetActivitiesInput input)
    {
        var query = await BuildScopedQueryAsync(input);

        var totalCount = await AsyncExecuter.CountAsync(query);
        var activities = await AsyncExecuter.ToListAsync(
            query
                // Deterministic and stable: blocks starting at the same instant keep their order across pages.
                .OrderByDescending(a => a.StartedAt)
                .ThenByDescending(a => a.Id)
                .PageBy(input.SkipCount, input.MaxResultCount)
        );

        var items = activities.Select(MapToDto).ToList();
        await AttachCapturesAsync(items);

        return new PagedResultDto<ActivityDto>(totalCount, items);
    }

    /// <summary>The block's full-size captures (screen/webcam). Thumbnails are a derived rendition of the screen
    /// capture and are excluded here; they are listed in <see cref="ActivityDto.Captures"/> instead.</summary>
    public async Task<List<ScreenshotDto>> GetScreenshotsAsync(Guid activityId)
    {
        await EnsureCanReadActivityAsync(activityId);

        var screenshots = await _screenshotRepository.GetListAsync(
            s => s.ActivityId == activityId && s.Kind != Screenshot.ThumbKind);

        return screenshots
            .OrderBy(s => s.Kind)
            .Select(s => new ScreenshotDto
            {
                Id = s.Id,
                ActivityId = s.ActivityId,
                Kind = s.Kind,
                Blurred = s.Blurred,
                CapturedAt = s.CapturedAt,
                SizeBytes = s.SizeBytes,
                ContentType = s.ContentType,
                CreationTime = s.CreationTime
            })
            .ToList();
    }

    public async Task<ScreenshotContentDto> GetScreenshotContentAsync(Guid id)
    {
        var screenshot = await _screenshotRepository.GetAsync(id);
        await EnsureCanReadActivityAsync(screenshot.ActivityId);

        var bytes = await _blobContainer.GetAllBytesOrNullAsync(screenshot.StorageUrl);
        if (bytes == null)
        {
            throw new EntityNotFoundException(typeof(Screenshot), id);
        }

        // The bytes decide: older servers stored every screen capture as image/png, whatever the agent sent.
        var contentType = CaptureImageFormat.DetectContentType(bytes) ?? screenshot.ContentType;
        return new ScreenshotContentDto
        {
            Bytes = bytes,
            ContentType = contentType,
            FileName = $"{screenshot.ActivityId}-{screenshot.Kind}{CaptureImageFormat.FileExtension(contentType)}"
        };
    }

    public async Task<ActivityDto> UpdateNoteAsync(Guid id, UpdateActivityNoteDto input)
    {
        var activity = await _activityRepository.GetAsync(id);

        if (activity.UserId == CurrentUser.GetId())
        {
            var age = UtcNow - AgentContextNormalizer.AsUtc(activity.StartedAt);
            if (age > NoteEditWindow && !await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage))
            {
                throw new AbpAuthorizationException(
                    $"The note of a block can be changed within {NoteEditWindow.TotalDays:0} days of its start.");
            }
        }
        else if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage))
        {
            if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
            {
                // Other members' blocks are hidden entirely from callers who cannot read them.
                throw new EntityNotFoundException(typeof(Activity), id);
            }

            throw new AbpAuthorizationException("Only the member who tracked a block, or a team manager, can change its note.");
        }

        activity.Description = AgentContextNormalizer.CleanNote(input.Description, UpdateActivityNoteDto.MaxDescriptionLength);
        await _activityRepository.UpdateAsync(activity, autoSave: true);

        return await MapToDtoWithCapturesAsync(activity);
    }

    /// <summary>The server's clock, as UTC.</summary>
    private DateTime UtcNow => AgentContextNormalizer.AsUtc(Clock.Now);

    private static string IngestLockName(Guid userId) => $"Tracker:ActivityIngest:{userId:N}";

    private Task<Activity?> FindOwnBlockAsync(Guid userId, Guid clientActivityId) =>
        _activityRepository.FirstOrDefaultAsync(x => x.UserId == userId && x.ClientActivityId == clientActivityId);

    /// <summary>The project a block is uploaded to. Only members who track time may upload: no membership, a
    /// <c>Client</c> role, or a deleted project are all refused as not a member (403, so agents refresh their
    /// projects and set the block aside if the project is really gone).</summary>
    private async Task<Project> GetUploadProjectAsync(Guid userId, Guid projectId)
    {
        var memberships = await _memberRepository.GetListAsync(m => m.ProjectId == projectId && m.UserId == userId);
        var project = memberships.Any(m => !TrackerRoles.IsClientProjectRole(m.Role))
            ? await _projectRepository.FindAsync(projectId)
            : null;

        if (project == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.NotProjectMember)
                .WithData("projectId", projectId);
        }

        return project;
    }

    /// <summary>
    /// The time rules of shared contract §1, in its order: a valid range, a project not archived before the block
    /// started, at most <see cref="MaxBlockLength"/> long, ending no later than <see cref="MaxFutureSkew"/> after the
    /// server's clock, starting within <see cref="ActivityIngestOptions.MaxBlockAge"/>, and no overlap of more than
    /// <see cref="AllowedOverlap"/> with the member's other blocks. Times are compared as UTC.
    /// </summary>
    private async Task ValidateBlockAsync(Guid userId, Project project, CreateActivityDto input)
    {
        var start = AgentContextNormalizer.AsUtc(input.StartedAt);
        var end = AgentContextNormalizer.AsUtc(input.EndedAt);
        if (end <= start)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityTimeRangeInvalid)
                .WithData("startedAt", input.StartedAt)
                .WithData("endedAt", input.EndedAt);
        }

        // Archiving stops new time, not time already recorded: blocks queued offline before it are still accepted.
        if (project.IsArchived && !(project.ArchivedAt.HasValue && AgentContextNormalizer.AsUtc(project.ArchivedAt.Value) > start))
        {
            throw new BusinessException(TrackerDomainErrorCodes.ProjectArchived)
                .WithData("projectId", project.Id);
        }

        var duration = end - start;
        if (duration > MaxBlockLength)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityTooLong)
                .WithData("durationMinutes", Math.Round(duration.TotalMinutes, 1))
                .WithData("maxMinutes", (int)MaxBlockLength.TotalMinutes);
        }

        var now = UtcNow;
        if (end > now + MaxFutureSkew)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityInFuture)
                .WithData("endedAt", input.EndedAt)
                .WithData("serverNow", now);
        }

        if (now - start > _ingestOptions.MaxBlockAge)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityTooOld)
                .WithData("startedAt", input.StartedAt)
                .WithData("serverNow", now)
                .WithData("maxAgeDays", Math.Round(_ingestOptions.MaxBlockAge.TotalDays, 1));
        }

        // Coarse DB filter, then the exact check. An overlap longer than AllowedOverlap implies both range bounds,
        // and a stored block (at most MaxBlockLength long) that reaches past this one's start began at most
        // MaxBlockLength earlier, which keeps the scan to a few rows of the (WorkspaceId, UserId, StartedAt) index.
        var earliestStart = start - MaxBlockLength;
        var overlapFrom = start + AllowedOverlap;
        var overlapTo = end - AllowedOverlap;
        var candidates = await _activityRepository.GetListAsync(a =>
            a.UserId == userId && a.StartedAt >= earliestStart && a.StartedAt < overlapTo && a.EndedAt > overlapFrom);

        var clash = candidates.FirstOrDefault(a =>
        {
            var otherStart = AgentContextNormalizer.AsUtc(a.StartedAt);
            var otherEnd = AgentContextNormalizer.AsUtc(a.EndedAt);
            return (otherEnd < end ? otherEnd : end) - (otherStart > start ? otherStart : start) > AllowedOverlap;
        });
        if (clash != null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityOverlaps)
                .WithData("startedAt", input.StartedAt)
                .WithData("endedAt", input.EndedAt);
        }
    }

    /// <summary>The block as stored. The capture policy is enforced server-side too: data the project does not
    /// allow is not stored, even if an outdated or misconfigured agent sends it; the time block itself is always kept.
    /// The optional context is normalized (an unusable value becomes null, never a rejection).</summary>
    private Activity BuildActivity(Guid userId, Project project, CreateActivityDto input)
    {
        var blockLength = AgentContextNormalizer.AsUtc(input.EndedAt) - AgentContextNormalizer.AsUtc(input.StartedAt);
        // Activity % and input integrity mean nothing when the project measures neither keyboard nor mouse.
        var measuresInput = project.AllowKeyboard || project.AllowMouse;
        var timeline = SanitizeTimeline(input.Timeline, project, blockLength);

        // The constructor enforces a non-empty client id and EndedAt > StartedAt.
        return new Activity(
            GuidGenerator.Create(),
            CurrentWorkspace.GetId(),
            userId,
            input.ProjectId,
            input.ClientActivityId,
            input.StartedAt,
            input.EndedAt
        )
        {
            Productivity = measuresInput ? input.Productivity : null,
            MouseClicks = project.AllowMouse ? Math.Max(0, input.MouseClicks) : 0,
            KeyboardHits = project.AllowKeyboard ? Math.Max(0, input.KeyboardHits) : 0,
            Description = AgentContextNormalizer.CleanNote(input.Description, UpdateActivityNoteDto.MaxDescriptionLength),
            ActiveWindowsJson = project.AllowActiveWindow ? SerializeWindows(input.ActiveWindows, project, blockLength) : "[]",
            RunningProgramsJson = project.AllowRunningPrograms ? SerializeRunningPrograms(input.RunningPrograms) : "[]",
            TimelineJson = timeline.Count == 0 ? "[]" : JsonSerializer.Serialize(timeline),
            Platform = AgentContextNormalizer.NormalizePlatform(input.Platform),
            OsVersion = AgentContextNormalizer.CleanText(input.OsVersion, ActivityConsts.MaxOsVersionLength),
            AgentVersion = AgentContextNormalizer.CleanText(input.AgentVersion, ActivityConsts.MaxAgentVersionLength),
            DeviceId = AgentContextNormalizer.NormalizeDeviceId(input.DeviceId),
            DeviceName = AgentContextNormalizer.CleanText(input.DeviceName, ActivityConsts.MaxDeviceNameLength),
            TimeZone = AgentContextNormalizer.NormalizeTimeZone(input.TimeZone),
            UtcOffsetMinutes = AgentContextNormalizer.NormalizeUtcOffset(input.UtcOffsetMinutes),
            IdleSeconds = ResolveIdleSeconds(input.IdleSeconds, timeline, blockLength, measuresInput),
            EndReason = AgentContextNormalizer.NormalizeEndReason(input.EndReason),
            ClockSkewSeconds = AgentContextNormalizer.ClockSkewSeconds(UtcNow, input.SentAt),
            InjectedInputEvents = measuresInput && input.InjectedInputEvents >= 0 ? input.InjectedInputEvents : null
        };
    }

    /// <summary>The reported idle seconds clamped to the block's length (a negative count is not a measurement);
    /// when the agent sent none, derived from the timeline's inactive buckets. Without input measurement (the
    /// project allows neither keyboard nor mouse) the timeline's activity flags say nothing about idling.</summary>
    private static int? ResolveIdleSeconds(
        int? reported, IReadOnlyCollection<ActivityMinuteDto> timeline, TimeSpan blockLength, bool measuresInput)
    {
        if (reported.HasValue)
        {
            return reported.Value >= 0
                ? Math.Min(reported.Value, (int)Math.Ceiling(blockLength.TotalSeconds))
                : null;
        }

        return measuresInput && timeline.Count > 0
            ? ActivityMinuteBuckets.IdleSeconds(timeline, blockLength)
            : null;
    }

    /// <summary>Serializes window info with over-long names/titles truncated (the list length itself is
    /// capped by validation on the DTO). Per-window usage is sanitized: seconds are clamped to the block's
    /// length, counts to non-negative, and input counts the project does not allow are dropped (stored as
    /// absent — "not captured" — rather than 0). Absent usage fields are omitted from the JSON.</summary>
    private static string SerializeWindows(List<WindowInfoDto>? windows, Project project, TimeSpan blockLength)
    {
        if (windows == null || windows.Count == 0)
        {
            return "[]";
        }

        var maxSeconds = Math.Max(0, (int)Math.Ceiling(blockLength.TotalSeconds));

        return JsonSerializer.Serialize(windows
            .Where(w => w != null)
            .Select(w => new WindowInfoDto
            {
                AppName = AgentContextNormalizer.Truncate(w.AppName, MaxAppNameLength),
                WindowTitle = AgentContextNormalizer.Truncate(w.WindowTitle, MaxWindowTitleLength),
                Seconds = w.Seconds.HasValue ? Math.Clamp(w.Seconds.Value, 0, maxSeconds) : null,
                KeyboardHits = project.AllowKeyboard && w.KeyboardHits.HasValue ? Math.Max(0, w.KeyboardHits.Value) : null,
                MouseClicks = project.AllowMouse && w.MouseClicks.HasValue ? Math.Max(0, w.MouseClicks.Value) : null
            })
            .ToList());
    }

    /// <summary>The running applications by name only — <c>[{"AppName":"chrome.exe"}]</c>, each once, in the
    /// order reported. Titles of background windows the member never focused are not stored (privacy), so every
    /// agent version yields the same shape; per-window usage belongs to the active windows.</summary>
    private static string SerializeRunningPrograms(List<WindowInfoDto>? programs)
    {
        if (programs == null || programs.Count == 0)
        {
            return "[]";
        }

        var names = programs
            .Select(p => AgentContextNormalizer.Truncate(p?.AppName?.Trim(), MaxAppNameLength))
            .Where(name => !string.IsNullOrEmpty(name))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .Select(name => new RunningProgramEntry(name!))
            .ToList();

        return names.Count == 0 ? "[]" : JsonSerializer.Serialize(names);
    }

    /// <summary>The stored shape of one running program.</summary>
    private sealed record RunningProgramEntry(string AppName);

    /// <summary>The per-minute timeline as stored: buckets outside the block (minute &lt; 0, or past its last
    /// minute plus one bucket of slack for a partial trailing minute) are dropped, duplicates keep the first
    /// entry, counts are clamped to non-negative, and data the project does not allow is blanked (counts
    /// zeroed, app name removed). The minute, its active flag and its active seconds are time data and always
    /// kept; active seconds outside 0..90 are not a measurement and are dropped.</summary>
    private static List<ActivityMinuteDto> SanitizeTimeline(
        List<ActivityMinuteDto>? timeline, Project project, TimeSpan blockLength)
    {
        if (timeline == null || timeline.Count == 0)
        {
            return new List<ActivityMinuteDto>();
        }

        var lastMinute = Math.Max(0, (int)Math.Ceiling(blockLength.TotalMinutes));

        return timeline
            .Where(m => m != null && m.Minute >= 0 && m.Minute <= lastMinute)
            .GroupBy(m => m.Minute)
            .Select(g => g.First())
            .OrderBy(m => m.Minute)
            .Select(m => new ActivityMinuteDto
            {
                Minute = m.Minute,
                KeyboardHits = project.AllowKeyboard ? Math.Max(0, m.KeyboardHits) : 0,
                MouseClicks = project.AllowMouse ? Math.Max(0, m.MouseClicks) : 0,
                Active = m.Active,
                AppName = project.AllowActiveWindow ? AgentContextNormalizer.Truncate(m.AppName, MaxAppNameLength) : null,
                ActiveSeconds = m.ActiveSeconds is >= 0 and <= ActivityConsts.MaxMinuteActiveSeconds ? m.ActiveSeconds : null
            })
            .ToList();
    }

    /// <summary>
    /// Decodes the captures the project allows. A capture that is missing, not valid base64, larger than its limit,
    /// neither JPEG nor PNG, or not allowed is skipped (logged) — a bad capture must never cost the time block or
    /// block the agent's offline queue. The screen capture and its thumbnail share the capture time the agent
    /// reported (when it lies within the block) and the blur flag; when the project requires blurred screenshots
    /// and the agent did not blur them, both are dropped. The webcam frame is taken at the block's end.
    /// </summary>
    private List<PendingCapture> DecodeCaptures(Project project, CreateActivityDto input)
    {
        var captures = new List<PendingCapture>(3);

        // Policy drops are routine with outdated agents (Information); invalid or oversized data is not (Warning).
        if (!project.AllowScreenshot)
        {
            LogDroppedCapture(input.ScreenshotPngBase64, Screenshot.ScreenKind, input, LogLevel.Information, "the project does not allow screenshots");
            LogDroppedCapture(input.ScreenshotThumbJpgBase64, Screenshot.ThumbKind, input, LogLevel.Information, "the project does not allow screenshots");
        }
        else if (project.BlurScreenshots && input.ScreenshotBlurred != true)
        {
            LogDroppedCapture(input.ScreenshotPngBase64, Screenshot.ScreenKind, input, LogLevel.Information, "the project requires blurred screenshots and the agent did not blur it");
            LogDroppedCapture(input.ScreenshotThumbJpgBase64, Screenshot.ThumbKind, input, LogLevel.Information, "the project requires blurred screenshots and the agent did not blur it");
        }
        else
        {
            var capturedAt = ResolveScreenCapturedAt(input);
            var blurred = input.ScreenshotBlurred == true;
            AddCaptureIfValid(captures, input, input.ScreenshotPngBase64, Screenshot.ScreenKind, MaxScreenCaptureBytes, capturedAt, blurred);
            AddCaptureIfValid(captures, input, input.ScreenshotThumbJpgBase64, Screenshot.ThumbKind, MaxThumbCaptureBytes, capturedAt, blurred);
        }

        if (project.AllowWebcam)
        {
            AddCaptureIfValid(captures, input, input.WebcamJpgBase64, Screenshot.WebcamKind, MaxWebcamCaptureBytes,
                AgentContextNormalizer.AsUtc(input.EndedAt), blurred: false);
        }
        else
        {
            LogDroppedCapture(input.WebcamJpgBase64, Screenshot.WebcamKind, input, LogLevel.Information, "the project does not allow the webcam");
        }

        return captures;
    }

    /// <summary>When the screen was captured: the agent's report if it lies within the block (±60 s of tolerance),
    /// otherwise the block's end.</summary>
    private static DateTime ResolveScreenCapturedAt(CreateActivityDto input)
    {
        var start = AgentContextNormalizer.AsUtc(input.StartedAt);
        var end = AgentContextNormalizer.AsUtc(input.EndedAt);
        if (input.ScreenshotCapturedAt.HasValue)
        {
            var reported = AgentContextNormalizer.AsUtc(input.ScreenshotCapturedAt.Value);
            if (reported >= start - CaptureTimeTolerance && reported <= end + CaptureTimeTolerance)
            {
                return reported;
            }
        }

        return end;
    }

    private void AddCaptureIfValid(
        List<PendingCapture> captures,
        CreateActivityDto input,
        string? base64,
        string kind,
        int maxBytes,
        DateTime capturedAt,
        bool blurred)
    {
        if (string.IsNullOrWhiteSpace(base64))
        {
            return;
        }

        // Refuse an oversized payload before decoding it (agents send unwrapped base64: 4 characters per 3 bytes).
        if (base64.Length > MaxBase64Length(maxBytes))
        {
            LogOversizedCapture(kind, input, (long)base64.Length * 3 / 4, maxBytes);
            return;
        }

        byte[] bytes;
        try
        {
            bytes = Convert.FromBase64String(base64);
        }
        catch (FormatException)
        {
            LogDroppedCapture(base64, kind, input, LogLevel.Warning, "the payload is not valid base64");
            return;
        }

        if (bytes.Length == 0)
        {
            return;
        }

        if (bytes.Length > maxBytes)
        {
            LogOversizedCapture(kind, input, bytes.Length, maxBytes);
            return;
        }

        var contentType = CaptureImageFormat.DetectContentType(bytes);
        if (contentType == null)
        {
            LogDroppedCapture(base64, kind, input, LogLevel.Warning, "it is neither a JPEG nor a PNG image");
            return;
        }

        captures.Add(new PendingCapture(kind, bytes, contentType, capturedAt, blurred));
    }

    private static int MaxBase64Length(int maxBytes) => (maxBytes + 2) / 3 * 4;

    private void LogOversizedCapture(string kind, CreateActivityDto input, long sizeBytes, int maxBytes)
    {
        Logger.LogWarning(
            "Dropping the {Kind} capture of block {ClientActivityId}: about {SizeBytes} bytes exceeds the {MaxBytes}-byte limit. The block is still recorded.",
            kind, input.ClientActivityId, sizeBytes, maxBytes);
    }

    private void LogDroppedCapture(string? base64, string kind, CreateActivityDto input, LogLevel level, string reason)
    {
        if (!string.IsNullOrWhiteSpace(base64))
        {
            Logger.Log(
                level,
                "Dropping the {Kind} capture of block {ClientActivityId} for project {ProjectId}: {Reason}. The block is still recorded.",
                kind, input.ClientActivityId, input.ProjectId, reason);
        }
    }

    /// <summary>
    /// Stores the block and its captures all-or-nothing, in a transactional unit of work of its own (so a failure
    /// cannot poison the caller's). The blobs are written first and the rows referencing them in one final save,
    /// so a failed blob write leaves no row behind; on any failure the blobs already written are deleted again and
    /// the error propagates, so the agent keeps the block queued and retries it. The one failure answered as success
    /// is the duplicate-upload race: when the unique (WorkspaceId, UserId, ClientActivityId) index refused the row
    /// because a concurrent upload of the same block was stored first, that block is returned (idempotency).
    /// </summary>
    private async Task<ActivityDto> StoreAsync(Activity activity, List<PendingCapture> captures)
    {
        var savedBlobs = new List<string>(captures.Count);
        var screenshots = new List<Screenshot>(captures.Count);
        try
        {
            using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);

            foreach (var capture in captures)
            {
                var blobName = $"activities/{activity.Id}/{capture.Kind}{CaptureImageFormat.FileExtension(capture.ContentType)}";
                await _blobContainer.SaveAsync(blobName, capture.Bytes, overrideExisting: true);
                savedBlobs.Add(blobName);

                screenshots.Add(new Screenshot(
                    GuidGenerator.Create(),
                    activity.WorkspaceId,
                    activity.Id,
                    blobName,
                    capture.CapturedAt,
                    capture.Bytes.Length,
                    capture.ContentType,
                    capture.Kind)
                {
                    Blurred = capture.Blurred
                });
            }

            await _activityRepository.InsertAsync(activity);
            await _screenshotRepository.InsertManyAsync(screenshots);
            await uow.CompleteAsync();
        }
        catch (Exception exception)
        {
            await DeleteBlobsAsync(savedBlobs);

            if (DatabaseErrors.MayBeUniqueViolation(exception))
            {
                var winner = await FindOwnBlockAsync(activity.UserId, activity.ClientActivityId);
                if (winner != null)
                {
                    Logger.LogInformation(
                        "Block {ClientActivityId} was stored by a concurrent upload; returning that one.",
                        activity.ClientActivityId);
                    return await MapToDtoWithCapturesAsync(winner);
                }
            }

            Logger.LogWarning(
                "Storing block {ClientActivityId} failed; nothing of it was kept and the agent will retry it.",
                activity.ClientActivityId);
            throw;
        }

        var dto = MapToDto(activity);
        dto.Captures = OrderCaptures(screenshots.Select(s => new ActivityCaptureRefDto
        {
            Id = s.Id,
            Kind = s.Kind,
            CapturedAt = s.CapturedAt,
            Blurred = s.Blurred
        }));
        return dto;
    }

    /// <summary>Best-effort removal of the blobs of a block that was not stored. With the database blob provider
    /// the transaction rollback has already removed them; an object store (S3, R2) needs the explicit delete.</summary>
    private async Task DeleteBlobsAsync(IReadOnlyCollection<string> blobNames)
    {
        if (blobNames.Count == 0)
        {
            return;
        }

        try
        {
            using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(), requiresNew: true);
            foreach (var blobName in blobNames)
            {
                await _blobContainer.DeleteAsync(blobName);
            }

            await uow.CompleteAsync();
        }
        catch (Exception exception)
        {
            Logger.LogWarning(exception,
                "Could not delete the captures of a block that was not stored; these blobs are orphaned: {BlobNames}",
                string.Join(", ", blobNames));
        }
    }

    private async Task<IQueryable<Activity>> BuildScopedQueryAsync(GetActivitiesInput input)
    {
        var query = await _activityRepository.GetQueryableAsync();

        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            if (input.UserId.HasValue)
            {
                query = query.Where(a => a.UserId == input.UserId.Value);
            }
        }
        else
        {
            // Regular members only ever see their own activity.
            var userId = CurrentUser.GetId();
            query = query.Where(a => a.UserId == userId);
        }

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (input.From.HasValue)
        {
            query = query.Where(a => a.StartedAt >= input.From.Value);
        }

        if (input.To.HasValue)
        {
            query = query.Where(a => a.StartedAt < input.To.Value);
        }

        return query;
    }

    private async Task EnsureCanReadActivityAsync(Guid activityId)
    {
        var activity = await _activityRepository.GetAsync(activityId);

        if (activity.UserId != CurrentUser.GetId() &&
            !await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            // Hide other members' captures entirely.
            throw new EntityNotFoundException(typeof(Activity), activityId);
        }
    }

    private ActivityDto MapToDto(Activity activity)
    {
        return new ActivityDto
        {
            Id = activity.Id,
            UserId = activity.UserId,
            ProjectId = activity.ProjectId,
            ClientActivityId = activity.ClientActivityId,
            StartedAt = activity.StartedAt,
            EndedAt = activity.EndedAt,
            Productivity = activity.Productivity,
            MouseClicks = activity.MouseClicks,
            KeyboardHits = activity.KeyboardHits,
            Description = activity.Description,
            ActiveWindowsJson = activity.ActiveWindowsJson,
            RunningProgramsJson = activity.RunningProgramsJson,
            TimelineJson = activity.TimelineJson,
            Platform = activity.Platform,
            OsVersion = activity.OsVersion,
            AgentVersion = activity.AgentVersion,
            DeviceId = activity.DeviceId,
            DeviceName = activity.DeviceName,
            TimeZone = activity.TimeZone,
            UtcOffsetMinutes = activity.UtcOffsetMinutes,
            IdleSeconds = activity.IdleSeconds,
            EndReason = activity.EndReason,
            ClockSkewSeconds = activity.ClockSkewSeconds,
            InjectedInputEvents = activity.InjectedInputEvents,
            CreationTime = activity.CreationTime
        };
    }

    private async Task<ActivityDto> MapToDtoWithCapturesAsync(Activity activity)
    {
        var dto = MapToDto(activity);
        await AttachCapturesAsync(new[] { dto });
        return dto;
    }

    /// <summary>Fills <see cref="ActivityDto.Captures"/> for the given blocks with a single query over their
    /// screenshot rows (id, kind, capture time and blur flag only; blob content is never loaded).</summary>
    private async Task AttachCapturesAsync(IReadOnlyCollection<ActivityDto> dtos)
    {
        if (dtos.Count == 0)
        {
            return;
        }

        var activityIds = dtos.Select(d => d.Id).ToList();
        var screenshots = await _screenshotRepository.GetQueryableAsync();
        var refs = await AsyncExecuter.ToListAsync(
            screenshots
                .Where(s => activityIds.Contains(s.ActivityId))
                .Select(s => new { s.Id, s.ActivityId, s.Kind, s.CapturedAt, s.Blurred }));

        var byActivity = refs.ToLookup(r => r.ActivityId);
        foreach (var dto in dtos)
        {
            dto.Captures = OrderCaptures(
                byActivity[dto.Id].Select(r => new ActivityCaptureRefDto
                {
                    Id = r.Id,
                    Kind = r.Kind,
                    CapturedAt = r.CapturedAt,
                    Blurred = r.Blurred
                }));
        }
    }

    /// <summary>Stable order for capture references: by kind ("screen", "thumb", "webcam"), then id.</summary>
    private static List<ActivityCaptureRefDto> OrderCaptures(IEnumerable<ActivityCaptureRefDto> captures) =>
        captures
            .OrderBy(c => c.Kind, StringComparer.Ordinal)
            .ThenBy(c => c.Id)
            .ToList();

    private record ExportActivitiesSnapshot(
        Guid WorkspaceId,
        DateTime SnapshotTimeUtc,
        IReadOnlyList<Activity> Activities,
        IReadOnlyDictionary<Guid, string> Projects,
        int TotalCount);

    private async Task<ExportActivitiesSnapshot> LoadExportActivitiesSnapshotAsync(ExportActivitiesInput input)
    {
        var currentWorkspaceId = CurrentWorkspace.GetId();
        var callerId = CurrentUser.GetId();

        // 1. Validate date bounds if provided
        if (input.From.HasValue && input.To.HasValue)
        {
            var fromUtc = AgentContextNormalizer.AsUtc(input.From.Value);
            var toUtc = AgentContextNormalizer.AsUtc(input.To.Value);
            if (toUtc <= fromUtc || toUtc - fromUtc > TimeSpan.FromDays(ExportActivitiesInput.MaxRangeDays))
            {
                throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid)
                    .WithData("maxDays", ExportActivitiesInput.MaxRangeDays);
            }
        }

        // 2. Validate maximum workload limit
        if (input.MaxResultCount.HasValue && (input.MaxResultCount.Value <= 0 || input.MaxResultCount.Value > ExportActivitiesInput.MaxAllowedResultCount))
        {
            throw new BusinessException(TrackerDomainErrorCodes.ExportExceedsLimit)
                .WithData("limit", ExportActivitiesInput.MaxAllowedResultCount);
        }
        var limit = input.MaxResultCount ?? ExportActivitiesInput.DefaultMaxResultCount;

        // 3. Authorization and User filtering
        var canViewAll = await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll);
        if (!canViewAll)
        {
            if (input.UserId.HasValue && input.UserId.Value != callerId)
            {
                throw new AbpAuthorizationException("Regular members cannot export other members' activities.");
            }
        }

        // 4. Project validation within current workspace
        if (input.ProjectId.HasValue)
        {
            var project = await _projectRepository.FindAsync(input.ProjectId.Value);
            if (project == null || project.WorkspaceId != currentWorkspaceId)
            {
                throw new EntityNotFoundException(typeof(Project), input.ProjectId.Value);
            }
        }

        // 5. Build Scoped Query
        var query = await _activityRepository.GetQueryableAsync();

        if (canViewAll)
        {
            if (input.UserId.HasValue)
            {
                query = query.Where(a => a.UserId == input.UserId.Value);
            }
        }
        else
        {
            query = query.Where(a => a.UserId == callerId);
        }

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (input.From.HasValue)
        {
            var fromUtc = AgentContextNormalizer.AsUtc(input.From.Value);
            query = query.Where(a => a.StartedAt >= fromUtc);
        }

        if (input.To.HasValue)
        {
            var toUtc = AgentContextNormalizer.AsUtc(input.To.Value);
            query = query.Where(a => a.StartedAt < toUtc);
        }

        // 6. Strict server-enforced workload limit (reject oversize/truncation)
        var totalCount = await AsyncExecuter.CountAsync(query);
        if (totalCount > limit)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ExportExceedsLimit)
                .WithData("totalCount", totalCount)
                .WithData("limit", limit);
        }

        // 7. Load activities ordered deterministically with hard-bounded Take(limit + 1)
        var activities = await AsyncExecuter.ToListAsync(
            query.OrderByDescending(a => a.StartedAt).ThenByDescending(a => a.Id).Take(limit + 1)
        );

        // 8. Strict post-materialization cardinality enforcement (reject candidate overflow past limit)
        if (activities.Count > limit)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ExportExceedsLimit)
                .WithData("totalCount", activities.Count)
                .WithData("limit", limit);
        }

        // 9. Derive honest count from materialized bounded snapshot
        totalCount = activities.Count;

        // 10. Load project titles
        var projectIds = activities.Select(a => a.ProjectId).Distinct().ToList();
        var projects = (await _projectRepository.GetListAsync(p => projectIds.Contains(p.Id)))
            .ToDictionary(p => p.Id, p => p.Title);

        return new ExportActivitiesSnapshot(currentWorkspaceId, DateTime.UtcNow, activities, projects, totalCount);
    }

    [RemoteService(IsEnabled = false)]
    public async Task<ActivityExportResultDto> ExportCsvAsync(ExportActivitiesInput input)
    {
        var snapshot = await LoadExportActivitiesSnapshotAsync(input);

        var csvBytes = CsvExportHelper.BuildActivitiesCsv(snapshot.Activities, snapshot.Projects);
        var fileName = $"activities-{snapshot.WorkspaceId:N}-{snapshot.SnapshotTimeUtc:yyyyMMddHHmmss}.csv";

        return new ActivityExportResultDto
        {
            Bytes = csvBytes,
            ContentType = "text/csv; charset=utf-8",
            FileName = fileName,
            TotalCount = snapshot.TotalCount,
            ExportedCount = snapshot.Activities.Count
        };
    }

    [RemoteService(IsEnabled = false)]
    public async Task<ActivityExportResultDto> ExportJsonAsync(ExportActivitiesInput input)
    {
        var snapshot = await LoadExportActivitiesSnapshotAsync(input);

        var jsonBytes = JsonExportHelper.BuildActivitiesJson(
            snapshot.WorkspaceId,
            snapshot.SnapshotTimeUtc,
            snapshot.Activities,
            snapshot.Projects);
        var fileName = $"activities-{snapshot.WorkspaceId:N}-{snapshot.SnapshotTimeUtc:yyyyMMddHHmmss}.json";

        return new ActivityExportResultDto
        {
            Bytes = jsonBytes,
            ContentType = "application/json; charset=utf-8",
            FileName = fileName,
            TotalCount = snapshot.TotalCount,
            ExportedCount = snapshot.Activities.Count
        };
    }
}
