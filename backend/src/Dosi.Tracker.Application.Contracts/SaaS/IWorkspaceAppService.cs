using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.SaaS;

public interface IWorkspaceAppService : IApplicationService
{
    /// <summary>The current workspace's subscription; null (HTTP 204) when it has none.</summary>
    Task<SubscriptionDto?> GetCurrentSubscriptionAsync();

    /// <summary>Anonymous: every plan, cheapest first (the sign-up page offers them).</summary>
    Task<ListResultDto<PlanDto>> GetAvailablePlansAsync();

    /// <summary>Anonymous sign-up: creates the account and a workspace it owns (or joins an invited workspace).
    /// <c>Tracker:EmailAlreadyRegistered</c> when the email has an account already.</summary>
    Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input);

    /// <summary>The workspace's settings; readable by every member.</summary>
    Task<WorkspaceSettingsDto> GetSettingsAsync();

    /// <summary>Workspace owners only.</summary>
    Task<WorkspaceSettingsDto> UpdateSettingsAsync(WorkspaceSettingsDto input);

    /// <summary>The signed-in user's own preferences (account-wide).</summary>
    Task<MyPreferencesDto> GetMyPreferencesAsync();

    Task<MyPreferencesDto> UpdateMyPreferencesAsync(MyPreferencesDto input);

    /// <summary><c>GET /api/app/workspaces/mine</c>: the workspaces the caller is an active member of, by name.</summary>
    Task<List<MyWorkspaceDto>> GetMineAsync();

    /// <summary><c>POST /api/app/workspaces</c>: creates a workspace owned by the caller.</summary>
    Task<CreatedWorkspaceDto> CreateAsync(CreateWorkspaceDto input);

    /// <summary><c>POST /api/app/workspaces/{id}/leave</c>. The last owner cannot leave.</summary>
    Task LeaveAsync(Guid id);

    /// <summary><c>POST /api/app/workspaces/{id}/archive</c>: Active owner of target workspace only. Idempotent.</summary>
    Task<WorkspaceDto> ArchiveAsync(Guid id);

    /// <summary><c>POST /api/app/workspaces/{id}/restore</c>: Active owner of target workspace only. Idempotent.</summary>
    Task<WorkspaceDto> RestoreAsync(Guid id);

    /// <summary><c>GET /api/app/workspaces/{id}/capture-retention</c>: Active member of target workspace only.</summary>
    Task<WorkspaceCaptureRetentionDto> GetCaptureRetentionAsync(Guid id);

    /// <summary><c>PUT /api/app/workspaces/{id}/capture-retention</c>: Active owner of target workspace only.</summary>
    Task<WorkspaceCaptureRetentionDto> UpdateCaptureRetentionAsync(Guid id, WorkspaceCaptureRetentionDto input);

    /// <summary><c>GET /api/app/workspaces/{id}/activity-retention</c>: Active member of target workspace only.</summary>
    Task<WorkspaceActivityRetentionDto> GetActivityRetentionAsync(Guid id);

    /// <summary><c>PUT /api/app/workspaces/{id}/activity-retention</c>: Active owner of target workspace only.</summary>
    Task<WorkspaceActivityRetentionDto> UpdateActivityRetentionAsync(Guid id, WorkspaceActivityRetentionDto input);

    /// <summary><c>GET /api/app/invitations/{token}</c> (anonymous).</summary>
    Task<InvitationInfoDto> GetInvitationAsync(string token);

    /// <summary><c>POST /api/app/invitations/{token}/accept</c>: the caller's email must be the invited one.</summary>
    Task<AcceptInvitationResultDto> AcceptInvitationAsync(string token);

    /// <summary><c>POST /api/app/invitations/{token}/decline</c>: the authenticated caller's email must match the invited one.</summary>
    Task DeclineInvitationAsync(string token);

    /// <summary><c>GET /api/app/invitations/mine</c>: the caller's pending invitations, matched by the account's email
    /// only once it is verified (an empty list otherwise).</summary>
    Task<MyInvitationsDto> GetMyInvitationsAsync();

    /// <summary><c>POST /api/app/invitations/mine/{id}/accept</c>: accepts one of <see cref="GetMyInvitationsAsync"/>
    /// without its token; 403 <c>Tracker:EmailNotConfirmed</c> while the account's email is unverified.</summary>
    Task<AcceptInvitationResultDto> AcceptMyInvitationAsync(Guid id);

    /// <summary><c>POST /api/app/invitations/mine/{id}/decline</c>: same rules as <see cref="AcceptMyInvitationAsync"/>.</summary>
    Task DeclineMyInvitationAsync(Guid id);
}
