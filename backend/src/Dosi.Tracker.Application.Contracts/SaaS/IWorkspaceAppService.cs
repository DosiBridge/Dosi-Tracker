using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.SaaS;

public interface IWorkspaceAppService : IApplicationService
{
    Task<SubscriptionDto> GetCurrentSubscriptionAsync();
    Task<ListResultDto<PlanDto>> GetAvailablePlansAsync();

    /// <summary>Anonymous SaaS signup: creates a tenant, seeds its admin user,
    /// and starts a subscription on the chosen plan.</summary>
    Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input);
}
