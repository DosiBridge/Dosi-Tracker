using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Platform;

/// <summary>
/// Platform-owner (host) console: cross-tenant metrics, revenue, and plan management. Every method
/// is host-only — a tenant-scoped caller is rejected. Cross-tenant reads run with the multi-tenancy
/// data filter disabled so the host sees every tenant's rows.
/// </summary>
public interface IPlatformAppService : IApplicationService
{
    Task<PlatformOverviewDto> GetOverviewAsync();

    Task<PagedResultDto<PlatformInvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input);

    Task<PagedResultDto<PlatformUserDto>> GetUsersAsync(PagedAndSortedResultRequestDto input);

    Task<ListResultDto<PlanDto>> GetPlansAsync();

    Task<PlanDto> CreatePlanAsync(CreateUpdatePlanDto input);

    Task<PlanDto> UpdatePlanAsync(Guid id, CreateUpdatePlanDto input);

    Task DeletePlanAsync(Guid id);
}
