using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Logging;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Authorization;
using Volo.Abp.BlobStoring;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Activities;

[Authorize]
public class ActivityAppService : ApplicationService, IActivityAppService
{
    /// <summary>Decoded capture payloads above this size are dropped (compressed PNG/JPEG
    /// screenshots are far below it; this only guards against abuse). Oversized/invalid
    /// captures are skipped, never fatal — the time block itself is always recorded.</summary>
    private const int MaxCaptureBytes = 10 * 1024 * 1024;

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IBlobContainer _blobContainer;

    public ActivityAppService(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IBlobContainer blobContainer)
    {
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _memberRepository = memberRepository;
        _blobContainer = blobContainer;
    }

    /// <summary>A capture decoded and ready to persist alongside its activity.</summary>
    private sealed record PendingCapture(string Kind, byte[] Bytes, string ContentType, string FileName);

    public async Task<ActivityDto> CreateAsync(CreateActivityDto input)
    {
        var existingActivity = await _activityRepository.FirstOrDefaultAsync(
            x => x.ClientActivityId == input.ClientActivityId);

        if (existingActivity != null)
        {
            // Idempotent: If agent retries, just return the existing one.
            return MapToDto(existingActivity);
        }

        var userId = CurrentUser.GetId(); // Throws if not authenticated; activities must belong to a real user.

        // Activities may only be recorded against a project the caller belongs to.
        var isMember = await _memberRepository.AnyAsync(
            m => m.ProjectId == input.ProjectId && m.UserId == userId);
        if (!isMember)
        {
            throw new AbpAuthorizationException("You are not a member of the target project.");
        }

        // Decode captures BEFORE the durable write: invalid/oversized captures are skipped
        // (logged), never committed-then-lost, and never fatal to the activity.
        var captures = new List<PendingCapture>(2);
        AddCaptureIfValid(captures, input.ScreenshotPngBase64, Screenshot.ScreenKind, "image/png", "screen.png");
        AddCaptureIfValid(captures, input.WebcamJpgBase64, Screenshot.WebcamKind, "image/jpeg", "webcam.jpg");

        var activity = new Activity(
            GuidGenerator.Create(),
            CurrentTenant.Id,
            userId,
            input.ProjectId,
            input.ClientActivityId,
            input.StartedAt,
            input.EndedAt
        )
        {
            Productivity = input.Productivity,
            MouseClicks = input.MouseClicks,
            KeyboardHits = input.KeyboardHits,
            Description = input.Description,
            ActiveWindowsJson = input.ActiveWindows != null ? JsonSerializer.Serialize(input.ActiveWindows) : "[]",
            RunningProgramsJson = input.RunningPrograms != null ? JsonSerializer.Serialize(input.RunningPrograms) : "[]"
        };

        try
        {
            // Separate UoW so a unique-index violation cannot poison the ambient unit of work,
            // and so the activity + its captures commit atomically (all-or-nothing).
            using var uow = UnitOfWorkManager.Begin(new Volo.Abp.Uow.AbpUnitOfWorkOptions(), requiresNew: true);

            await _activityRepository.InsertAsync(activity, autoSave: true);

            foreach (var capture in captures)
            {
                var blobName = $"activities/{activity.Id}/{capture.FileName}";
                await _blobContainer.SaveAsync(blobName, capture.Bytes, overrideExisting: true);
                await _screenshotRepository.InsertAsync(new Screenshot(
                    GuidGenerator.Create(),
                    CurrentTenant.Id,
                    activity.Id,
                    blobName,
                    activity.EndedAt,
                    capture.Bytes.Length,
                    capture.ContentType,
                    capture.Kind));
            }

            await uow.CompleteAsync();
        }
        catch (Exception)
        {
            // A concurrent retry may have won the unique (TenantId, ClientActivityId) race — stay idempotent.
            var winner = await _activityRepository.FirstOrDefaultAsync(
                x => x.ClientActivityId == input.ClientActivityId);
            if (winner != null)
            {
                return MapToDto(winner);
            }

            throw;
        }

        return MapToDto(activity);
    }

    public async Task<PagedResultDto<ActivityDto>> GetListAsync(GetActivitiesInput input)
    {
        var query = await BuildScopedQueryAsync(input);

        var totalCount = await AsyncExecuter.CountAsync(query);
        var activities = await AsyncExecuter.ToListAsync(
            query
                .OrderByDescending(a => a.StartedAt) // Deterministic order; Skip/Take without OrderBy is undefined.
                .PageBy(input.SkipCount, input.MaxResultCount)
        );

        return new PagedResultDto<ActivityDto>(
            totalCount,
            activities.Select(MapToDto).ToList()
        );
    }

    public async Task<List<ScreenshotDto>> GetScreenshotsAsync(Guid activityId)
    {
        await EnsureCanReadActivityAsync(activityId);

        var screenshots = await _screenshotRepository.GetListAsync(s => s.ActivityId == activityId);

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

        return new ScreenshotContentDto
        {
            Bytes = bytes,
            ContentType = screenshot.ContentType,
            FileName = $"{screenshot.ActivityId}-{screenshot.Kind}{(screenshot.ContentType == "image/png" ? ".png" : ".jpg")}"
        };
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

    /// <summary>Decodes a capture and queues it for atomic persistence with the activity.
    /// Missing/invalid/oversized captures are skipped (logged) — a bad screenshot must never
    /// cost the whole time block or block the agent's offline queue.</summary>
    private void AddCaptureIfValid(
        List<PendingCapture> captures, string? base64, string kind, string contentType, string fileName)
    {
        if (string.IsNullOrWhiteSpace(base64))
        {
            return;
        }

        byte[] bytes;
        try
        {
            bytes = Convert.FromBase64String(base64);
        }
        catch (FormatException)
        {
            Logger.LogWarning("Dropping {Kind} capture for activity: payload is not valid base64.", kind);
            return;
        }

        if (bytes.Length == 0)
        {
            return;
        }

        if (bytes.Length > MaxCaptureBytes)
        {
            Logger.LogWarning(
                "Dropping {Kind} capture for activity: {SizeBytes} bytes exceeds the {MaxBytes}-byte limit.",
                kind, bytes.Length, MaxCaptureBytes);
            return;
        }

        captures.Add(new PendingCapture(kind, bytes, contentType, fileName));
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
            CreationTime = activity.CreationTime
        };
    }
}
