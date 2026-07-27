using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Billing;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;

namespace Dosi.Tracker.Platform;

[Authorize]
public class PlatformAppService : TrackerAppService, IPlatformAppService
{
    private readonly ITenantRepository _tenantRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IGuidGenerator _guidGenerator;

    public PlatformAppService(
        ITenantRepository tenantRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Invoice, Guid> invoiceRepository,
        IDataFilter dataFilter,
        IGuidGenerator guidGenerator)
    {
        _tenantRepository = tenantRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceRepository = invoiceRepository;
        _dataFilter = dataFilter;
        _guidGenerator = guidGenerator;
    }

    public async Task<PlatformOverviewDto> GetOverviewAsync()
    {
        EnsureHost();

        var tenantCount = (int)await _tenantRepository.GetCountAsync();
        var plans = await _planRepository.GetListAsync();

        // Host aggregation: see every tenant's subscriptions and invoices, not just the host's.
        using (_dataFilter.Disable<IMultiTenant>())
        {
            var subscriptions = await _subscriptionRepository.GetListAsync();
            var invoices = await _invoiceRepository.GetListAsync();

            return new PlatformOverviewDto
            {
                TenantCount = tenantCount,
                ActiveSubscriptions = subscriptions.Count(s => s.Status == "active"),
                TrialingSubscriptions = subscriptions.Count(s => s.Status == "trialing"),
                TotalInvoiced = invoices.Sum(i => i.Amount),
                PendingInvoices = invoices.Count(i => i.Status == "pending"),
                Plans = plans
                    .OrderBy(p => p.PricePerUser)
                    .Select(p => new PlanSubscriberDto
                    {
                        PlanId = p.Id,
                        Name = p.Name,
                        PricePerUser = p.PricePerUser,
                        SubscriberCount = subscriptions.Count(s => s.PlanId == p.Id)
                    })
                    .ToList()
            };
        }
    }

    public async Task<PagedResultDto<PlatformInvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input)
    {
        EnsureHost();

        var tenantNames = (await _tenantRepository.GetListAsync()).ToDictionary(t => t.Id, t => t.Name);

        using (_dataFilter.Disable<IMultiTenant>())
        {
            var all = (await _invoiceRepository.GetListAsync()).OrderByDescending(i => i.DueDate).ToList();
            var page = all
                .Skip(input.SkipCount)
                .Take(input.MaxResultCount)
                .Select(i => new PlatformInvoiceDto
                {
                    Id = i.Id,
                    TenantId = i.TenantId,
                    TenantName = i.TenantId.HasValue && tenantNames.TryGetValue(i.TenantId.Value, out var n) ? n : "Host",
                    Amount = i.Amount,
                    Status = i.Status,
                    DueDate = i.DueDate
                })
                .ToList();

            return new PagedResultDto<PlatformInvoiceDto>(all.Count, page);
        }
    }

    public async Task<ListResultDto<PlanDto>> GetPlansAsync()
    {
        EnsureHost();
        var plans = await _planRepository.GetListAsync();
        return new ListResultDto<PlanDto>(
            ObjectMapper.Map<List<Plan>, List<PlanDto>>(plans.OrderBy(p => p.PricePerUser).ToList()));
    }

    public async Task<PlanDto> CreatePlanAsync(CreateUpdatePlanDto input)
    {
        EnsureHost();
        var plan = new Plan(_guidGenerator.Create(), input.Name, input.PricePerUser, input.MaxSeats, input.TrialDays);
        await _planRepository.InsertAsync(plan, autoSave: true);
        return ObjectMapper.Map<Plan, PlanDto>(plan);
    }

    public async Task<PlanDto> UpdatePlanAsync(Guid id, CreateUpdatePlanDto input)
    {
        EnsureHost();
        var plan = await _planRepository.GetAsync(id);
        plan.Name = input.Name;
        plan.PricePerUser = input.PricePerUser;
        plan.MaxSeats = input.MaxSeats;
        plan.TrialDays = input.TrialDays;
        await _planRepository.UpdateAsync(plan, autoSave: true);
        return ObjectMapper.Map<Plan, PlanDto>(plan);
    }

    public async Task DeletePlanAsync(Guid id)
    {
        EnsureHost();
        await _planRepository.DeleteAsync(id);
    }

    /// <summary>The platform console is for host administrators; a tenant-scoped caller is refused.</summary>
    private void EnsureHost()
    {
        if (CurrentTenant.Id != null)
        {
            throw new AbpAuthorizationException("The platform console is available to host administrators only.");
        }
    }
}
