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

    /// <summary><c>GET /api/app/invitations/{token}</c> (anonymous).</summary>
    Task<InvitationInfoDto> GetInvitationAsync(string token);

    /// <summary><c>POST /api/app/invitations/{token}/accept</c>: the caller's email must be the invited one.</summary>
    Task<AcceptInvitationResultDto> AcceptInvitationAsync(string token);
}
