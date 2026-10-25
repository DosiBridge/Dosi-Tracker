using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Auditing;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Uow;
using Volo.Abp.Users;
using Volo.Abp.Validation;

namespace Dosi.Tracker.Activities;

/// <inheritdoc cref="IActivityErasureAppService"/>
[Authorize]
[RemoteService(IsEnabled = false)]
public class ActivityErasureAppService : TrackerAppService, IActivityErasureAppService
{
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<ActivityErasure, Guid> _erasureRepository;
    private readonly TrackedDataEraser _eraser;
    private readonly WorkspaceMembershipManager _memberships;
    private readonly WorkspaceSettingsManager _workspaceSettings;

    public ActivityErasureAppService(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<ActivityErasure, Guid> erasureRepository,
        TrackedDataEraser eraser,
        WorkspaceMembershipManager memberships,
        WorkspaceSettingsManager workspaceSettings)
    {
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _erasureRepository = erasureRepository;
        _eraser = eraser;
        _memberships = memberships;
        _workspaceSettings = workspaceSettings;
    }

    [Authorize(TrackerPermissions.Activities.Delete)]
    public async Task<ActivityErasureDto> DeleteBlockAsync(Guid id, DeleteTrackedDataDto input)
    {
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        var activity = await _activityRepository.GetAsync(id);
        await EnsureMayDeleteDataOfAsync(activity.UserId);

        var record = await _eraser.EraseBlockAsync(activity, input.Reason);
        await uow.CompleteAsync();

        Audit($"Deleted the time block {activity.Id} of user {activity.UserId} with {record.CaptureCount} capture(s).");
        return ToDto(record);
    }

    public async Task<ActivityErasureDto> DeleteCaptureAsync(Guid captureId, DeleteCaptureDto input)
    {
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        var capture = await _screenshotRepository.FindAsync(captureId)
                      ?? throw new EntityNotFoundException(typeof(Screenshot), captureId);
        var activity = await _activityRepository.GetAsync(capture.ActivityId);

        var isOwn = activity.UserId == CurrentUser.GetId();
        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.Delete))
        {
            if (!isOwn)
            {
                await EnsureMayDeleteDataOfAsync(activity.UserId);
                if (string.IsNullOrWhiteSpace(input.Reason) || input.Reason.Trim().Length < ActivityErasureLimits.MinReasonLength)
                {
                    throw new AbpValidationException(new List<ValidationResult>
                    {
                        new("Give the reason for deleting someone else's capture.", new[] { nameof(DeleteCaptureDto.Reason) })
                    });
                }
            }
        }
        else if (isOwn)
        {
            await EnsureWithinMemberWindowAsync(capture);
        }
        else if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            throw new AbpAuthorizationException("Only workspace owners and managers can delete another member's captures.");
        }
        else
        {
            throw new EntityNotFoundException(typeof(Screenshot), captureId); // Hidden, like the capture itself.
        }

        var group = await _eraser.GetCaptureGroupAsync(capture);
        var record = await _eraser.EraseCapturesAsync(activity, group, input.Reason);
        await uow.CompleteAsync();

        Audit($"Deleted {group.Count} capture(s) of the time block {activity.Id} of user {activity.UserId}.");
        return ToDto(record);
    }

    public async Task<PagedResultDto<ActivityErasureDto>> GetErasuresAsync(PagedResultRequestDto input)
    {
        var query = await _erasureRepository.GetQueryableAsync();
        if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.Delete))
        {
            var userId = CurrentUser.GetId();
            query = query.Where(e => e.UserId == userId);
        }

        var totalCount = await AsyncExecuter.CountAsync(query);
        var page = await AsyncExecuter.ToListAsync(query
            .OrderByDescending(e => e.CreationTime)
            .ThenByDescending(e => e.Id)
            .PageBy(input.SkipCount, input.MaxResultCount));

        return new PagedResultDto<ActivityErasureDto>(totalCount, page.Select(ToDto).ToList());
    }

    /// <summary>Only an owner deletes an owner's data (as for an owner's devices or memberships).</summary>
    private async Task EnsureMayDeleteDataOfAsync(Guid targetUserId)
    {
        var callerId = CurrentUser.GetId();
        if (targetUserId == callerId)
        {
            return;
        }

        var workspaceId = CurrentWorkspace.GetId();
        var target = await _memberships.FindActiveAsync(workspaceId, targetUserId);
        if (target is { IsOwner: true } && (await _memberships.FindActiveAsync(workspaceId, callerId)) is not { IsOwner: true })
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed).WithData("userId", targetUserId);
        }
    }

    /// <summary>A member deletes their own capture only within the workspace's window after it was uploaded.</summary>
    private async Task EnsureWithinMemberWindowAsync(Screenshot capture)
    {
        var windowMinutes = await _workspaceSettings.GetMemberCaptureDeleteMinutesAsync();
        var uploadedAt = AgentContextNormalizer.AsUtc(capture.CreationTime);
        var now = AgentContextNormalizer.AsUtc(Clock.Now);
        if (windowMinutes <= 0 || now - uploadedAt > TimeSpan.FromMinutes(windowMinutes))
        {
            throw new BusinessException(TrackerDomainErrorCodes.CaptureDeleteNotAllowed)
                .WithData("windowMinutes", windowMinutes);
        }
    }

    private void Audit(string comment)
    {
        LazyServiceProvider.LazyGetService<IAuditingManager>()?.Current?.Log.Comments.Add(comment);
    }

    private static ActivityErasureDto ToDto(ActivityErasure erasure) => new()
    {
        Id = erasure.Id,
        ActivityId = erasure.ActivityId,
        UserId = erasure.UserId,
        ProjectId = erasure.ProjectId,
        StartedAt = erasure.StartedAt,
        EndedAt = erasure.EndedAt,
        Scope = erasure.Scope,
        CaptureKinds = erasure.CaptureKinds,
        CaptureCount = erasure.CaptureCount,
        Reason = erasure.Reason,
        DeletedBy = erasure.CreatorId,
        DeletionTime = erasure.CreationTime
    };
}
