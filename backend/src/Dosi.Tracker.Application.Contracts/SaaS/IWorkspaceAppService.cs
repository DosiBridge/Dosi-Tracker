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

    /// <summary>Anonymous SaaS signup: creates a tenant, seeds its admin user,
    /// and starts a subscription on the chosen plan.</summary>
    Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input);

    /// <summary>The workspace's settings; readable by every member (the default time zone decides their reports' days).</summary>
    Task<WorkspaceSettingsDto> GetSettingsAsync();

    /// <summary>Workspace owners only.</summary>
    Task<WorkspaceSettingsDto> UpdateSettingsAsync(WorkspaceSettingsDto input);

    /// <summary>The signed-in user's own preferences.</summary>
    Task<MyPreferencesDto> GetMyPreferencesAsync();

    Task<MyPreferencesDto> UpdateMyPreferencesAsync(MyPreferencesDto input);
}
