using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;

namespace Dosi.Tracker.Platform;

/// <summary>Host console. Requires the host-only <see cref="TrackerPermissions.Platform.Default"/> permission
/// (granted to the host "admin" role by ABP's permission seeding) AND a host-side caller.</summary>
[Authorize(TrackerPermissions.Platform.Default)]
public class PlatformAppService : TrackerAppService, IPlatformAppService
{
    private readonly ITenantRepository _tenantRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IGuidGenerator _guidGenerator;

    public PlatformAppService(
        ITenantRepository tenantRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Invoice, Guid> invoiceRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IIdentityUserRepository userRepository,
        IDataFilter dataFilter,
        IGuidGenerator guidGenerator)
    {
        _tenantRepository = tenantRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceRepository = invoiceRepository;
        _memberRepository = memberRepository;
        _userRepository = userRepository;
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
            var members = await _memberRepository.GetListAsync();

            // Occupied (billable) seats per tenant = distinct members holding a project seat there.
            var seatsByTenant = members
                .GroupBy(m => m.TenantId)
                .ToDictionary(g => g.Key, g => g.Select(m => m.UserId).Distinct().Count());

            decimal mrr = 0m;
            var paidSeats = 0;
            // Only paying (active) subscriptions count as recurring revenue; a trial pays nothing yet.
            foreach (var sub in subscriptions.Where(s => s.Status == InvoiceGenerationWorker.BillableStatus))
            {
                var plan = plans.FirstOrDefault(p => p.Id == sub.PlanId);
                if (plan is null)
                {
                    continue;
                }
                var seats = Math.Max(1, seatsByTenant.GetValueOrDefault(sub.TenantId));
                mrr += plan.PricePerUser * seats;
                paidSeats += seats;
            }

            return new PlatformOverviewDto
            {
                TenantCount = tenantCount,
                ActiveSubscriptions = subscriptions.Count(s => s.Status == "active"),
                TrialingSubscriptions = subscriptions.Count(s => s.Status == "trialing"),
                Mrr = mrr,
                PaidSeats = paidSeats,
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

        using (_dataFilter.Disable<IMultiTenant>())
        {
            // Count and page in the database; only the requested page is materialized.
            var query = await _invoiceRepository.GetQueryableAsync();
            var totalCount = await AsyncExecuter.CountAsync(query);
            var page = await AsyncExecuter.ToListAsync(query
                .OrderByDescending(i => i.DueDate)
                .ThenBy(i => i.Id) // stable order across pages
                .PageBy(input.SkipCount, input.MaxResultCount));

            var tenantNames = await GetTenantNamesAsync();

            var items = page
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

            return new PagedResultDto<PlatformInvoiceDto>(totalCount, items);
        }
    }

    public async Task<PagedResultDto<PlatformUserDto>> GetUsersAsync(PagedAndSortedResultRequestDto input)
    {
        EnsureHost();

        var tenantNames = (await _tenantRepository.GetListAsync()).ToDictionary(t => t.Id, t => t.Name);

        using (_dataFilter.Disable<IMultiTenant>())
        {
            var count = await _userRepository.GetCountAsync();
            var users = await _userRepository.GetListAsync(
                sorting: nameof(IdentityUser.UserName),
                maxResultCount: input.MaxResultCount,
                skipCount: input.SkipCount);

            var items = users.Select(u => new PlatformUserDto
            {
                Id = u.Id,
                UserName = u.UserName,
                Email = u.Email,
                Name = u.Name,
                TenantId = u.TenantId,
                TenantName = u.TenantId.HasValue && tenantNames.TryGetValue(u.TenantId.Value, out var n) ? n : "Host"
            }).ToList();

            return new PagedResultDto<PlatformUserDto>(count, items);
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

        // A plan still referenced by any tenant's subscription cannot be removed (it would orphan
        // seat limits, billing and the overview's plan counts). Check across every tenant.
        long inUse;
        using (_dataFilter.Disable<IMultiTenant>())
        {
            inUse = await _subscriptionRepository.CountAsync(s => s.PlanId == id);
        }

        if (inUse > 0)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanInUse)
                .WithData("planId", id)
                .WithData("subscriptions", inUse);
        }

        await _planRepository.DeleteAsync(id);
    }

    private async Task<Dictionary<Guid, string>> GetTenantNamesAsync()
    {
        return (await _tenantRepository.GetListAsync()).ToDictionary(t => t.Id, t => t.Name);
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
