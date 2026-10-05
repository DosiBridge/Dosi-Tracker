using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Permissions;
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
using Volo.Abp.Validation;

namespace Dosi.Tracker.Platform;

/// <summary>Host console. Requires the host-only <see cref="TrackerPermissions.Platform.Default"/> permission
/// (granted to the host "admin" role by ABP's permission seeding) AND a host-side caller.</summary>
[Authorize(TrackerPermissions.Platform.Default)]
public class PlatformAppService : TrackerAppService, IPlatformAppService
{
    /// <summary>Workspace name shown for host-side rows (no tenant).</summary>
    public const string HostName = "Host";

    /// <summary>Workspace name shown for rows of a tenant that no longer exists.</summary>
    public const string DeletedWorkspaceName = "Deleted workspace";

    private readonly ITenantRepository _tenantRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IGuidGenerator _guidGenerator;
    private readonly SeatCounter _seatCounter;
    private readonly SubscriptionManager _subscriptionManager;

    public PlatformAppService(
        ITenantRepository tenantRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Invoice, Guid> invoiceRepository,
        IIdentityUserRepository userRepository,
        IDataFilter dataFilter,
        IGuidGenerator guidGenerator,
        SeatCounter seatCounter,
        SubscriptionManager subscriptionManager)
    {
        _tenantRepository = tenantRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceRepository = invoiceRepository;
        _userRepository = userRepository;
        _dataFilter = dataFilter;
        _guidGenerator = guidGenerator;
        _seatCounter = seatCounter;
        _subscriptionManager = subscriptionManager;
    }

    public async Task<PlatformOverviewDto> GetOverviewAsync()
    {
        EnsureHost();

        var tenantNames = await GetTenantNamesAsync();
        var plans = await _planRepository.GetListAsync();

        // Host aggregation: see every tenant's subscriptions and invoices, not just the host's.
        using (_dataFilter.Disable<IMultiTenant>())
        {
            // Customers only: a deleted (or unknown) workspace's subscription is neither revenue nor a subscriber.
            var subscriptions = (await _subscriptionRepository.GetListAsync())
                .Where(s => s.TenantId.HasValue && tenantNames.ContainsKey(s.TenantId.Value))
                .ToList();
            var invoices = await _invoiceRepository.GetListAsync();

            // Occupied (billable) seats per tenant: the same count the invoice and the seat limit use.
            var seatsByTenant = await _seatCounter.CountByTenantAsync();

            decimal mrr = 0m;
            var paidSeats = 0;
            // Only paying (active) subscriptions count as recurring revenue; a trial pays nothing yet.
            foreach (var sub in subscriptions.Where(s => s.Status == InvoiceGenerationWorker.BillableStatus))
            {
                var plan = plans.FirstOrDefault(p => p.Id == sub.PlanId);
                if (plan is null || plan.IsFree())
                {
                    continue; // An unknown or free plan pays nothing: neither revenue nor paid seats.
                }
                var seats = Math.Max(1, seatsByTenant.GetValueOrDefault(sub.TenantId!.Value));
                mrr += plan.PricePerUser * seats;
                paidSeats += seats;
            }

            return new PlatformOverviewDto
            {
                TenantCount = tenantNames.Count,
                ActiveSubscriptions = subscriptions.Count(s => s.Status == Subscription.ActiveStatus),
                TrialingSubscriptions = subscriptions.Count(s => s.Status == Subscription.TrialingStatus),
                Mrr = mrr,
                PaidSeats = paidSeats,
                TotalInvoiced = invoices.Sum(i => i.Amount),
                PendingInvoices = invoices.Count(i => i.Status == Invoice.PendingStatus),
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
                .Select(i => ToPlatformInvoiceDto(i, tenantNames))
                .ToList();

            return new PagedResultDto<PlatformInvoiceDto>(totalCount, items);
        }
    }

    public async Task<PagedResultDto<PlatformUserDto>> GetUsersAsync(PagedAndSortedResultRequestDto input)
    {
        EnsureHost();

        var tenantNames = await GetTenantNamesAsync();

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
                TenantName = ResolveTenantName(u.TenantId, tenantNames)
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
        var name = input.Name.Trim();
        await CheckPlanNameIsFreeAsync(name, exceptPlanId: null);

        var plan = new Plan(_guidGenerator.Create(), name, input.PricePerUser, input.MaxSeats, input.TrialDays);
        await _planRepository.InsertAsync(plan, autoSave: true);
        return ObjectMapper.Map<Plan, PlanDto>(plan);
    }

    public async Task<PlanDto> UpdatePlanAsync(Guid id, CreateUpdatePlanDto input)
    {
        EnsureHost();
        var plan = await _planRepository.GetAsync(id);
        var name = input.Name.Trim();
        await CheckPlanNameIsFreeAsync(name, exceptPlanId: id);

        plan.Name = name;
        plan.PricePerUser = input.PricePerUser;
        plan.MaxSeats = input.MaxSeats;
        plan.TrialDays = input.TrialDays;
        await _planRepository.UpdateAsync(plan, autoSave: true);
        return ObjectMapper.Map<Plan, PlanDto>(plan);
    }

    public async Task DeletePlanAsync(Guid id)
    {
        EnsureHost();

        // A plan still referenced by any workspace's subscription cannot be removed (it would orphan
        // seat limits, billing and the overview's plan counts). Check across every existing workspace.
        var tenantNames = await GetTenantNamesAsync();
        List<Subscription> subscribers;
        using (_dataFilter.Disable<IMultiTenant>())
        {
            subscribers = await _subscriptionRepository.GetListAsync(s => s.PlanId == id);
        }

        var inUse = subscribers.Count(s => s.TenantId.HasValue && tenantNames.ContainsKey(s.TenantId.Value));
        if (inUse > 0)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanInUse)
                .WithData("planId", id)
                .WithData("subscriptions", inUse);
        }

        await _planRepository.DeleteAsync(id);
    }

    public async Task<ListResultDto<TenantSubscriptionDto>> GetTenantSubscriptionsAsync()
    {
        EnsureHost();

        var tenants = await _tenantRepository.GetListAsync();
        var plans = (await _planRepository.GetListAsync()).ToDictionary(p => p.Id);

        using (_dataFilter.Disable<IMultiTenant>())
        {
            var subscriptions = (await _subscriptionRepository.GetListAsync(s => s.TenantId != null))
                .GroupBy(s => s.TenantId!.Value)
                .ToDictionary(g => g.Key, g => g.OrderByDescending(s => s.CreationTime).First());
            var seatsByTenant = await _seatCounter.CountByTenantAsync();

            var items = tenants
                .OrderBy(t => t.Name)
                .Select(t =>
                {
                    var subscription = subscriptions.GetValueOrDefault(t.Id);
                    var plan = subscription == null ? null : plans.GetValueOrDefault(subscription.PlanId);
                    return new TenantSubscriptionDto
                    {
                        TenantId = t.Id,
                        TenantName = t.Name,
                        SubscriptionId = subscription?.Id,
                        PlanId = subscription?.PlanId,
                        PlanName = plan?.Name,
                        Status = subscription?.Status,
                        TrialEndsAt = subscription?.TrialEndsAt,
                        SeatsUsed = seatsByTenant.GetValueOrDefault(t.Id),
                        MaxSeats = plan?.MaxSeats
                    };
                })
                .ToList();

            return new ListResultDto<TenantSubscriptionDto>(items);
        }
    }

    public async Task<SubscriptionDto> ChangeTenantPlanAsync(Guid tenantId, Guid planId)
    {
        EnsureHost();
        await _tenantRepository.GetAsync(tenantId, includeDetails: false);

        var plan = await _planRepository.FindAsync(planId);
        if (plan == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                .WithData("planId", planId);
        }

        using (CurrentTenant.Change(tenantId))
        {
            var subscription = await _subscriptionManager.ChangePlanAsync(plan);
            return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
        }
    }

    /// <inheritdoc />
    /// <remarks><paramref name="tenantId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?tenantId=</c>, the route of the shared contract) instead of a <c>/{tenantId}</c> route segment.</remarks>
    public async Task<SubscriptionDto> ActivateSubscriptionAsync([Optional] Guid tenantId)
    {
        EnsureHost();
        await _tenantRepository.GetAsync(tenantId, includeDetails: false);

        using (CurrentTenant.Change(tenantId))
        {
            var subscription = await _subscriptionManager.ActivateAsync();
            return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
        }
    }

    /// <inheritdoc />
    /// <remarks><paramref name="tenantId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?tenantId=</c>) instead of a <c>/{tenantId}</c> route segment.</remarks>
    public async Task<SubscriptionDto> SuspendSubscriptionAsync([Optional] Guid tenantId)
    {
        EnsureHost();
        await _tenantRepository.GetAsync(tenantId, includeDetails: false);

        using (CurrentTenant.Change(tenantId))
        {
            var subscription = await _subscriptionManager.SuspendAsync();
            return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
        }
    }

    /// <inheritdoc />
    /// <remarks><paramref name="invoiceId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?invoiceId=</c>, the route of the shared contract) instead of a <c>/{invoiceId}</c> route segment.</remarks>
    public async Task<PlatformInvoiceDto> MarkInvoicePaidAsync([Optional] Guid invoiceId)
    {
        EnsureHost();

        Invoice invoice;
        using (_dataFilter.Disable<IMultiTenant>())
        {
            invoice = await _invoiceRepository.GetAsync(invoiceId);
        }

        using (CurrentTenant.Change(invoice.TenantId))
        {
            await _subscriptionManager.MarkInvoicePaidAsync(invoice);
        }

        return ToPlatformInvoiceDto(invoice, await GetTenantNamesAsync());
    }

    /// <summary>Plan names are unique (case-insensitively): sign-up can choose a plan by name.</summary>
    private async Task CheckPlanNameIsFreeAsync(string name, Guid? exceptPlanId)
    {
        var lowered = name.ToLowerInvariant();
        var query = (await _planRepository.GetQueryableAsync()).Where(p => p.Name.ToLower() == lowered);
        if (exceptPlanId.HasValue)
        {
            var id = exceptPlanId.Value;
            query = query.Where(p => p.Id != id);
        }

        if (await AsyncExecuter.AnyAsync(query))
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new($"A plan named '{name}' already exists.", new[] { nameof(CreateUpdatePlanDto.Name) })
            });
        }
    }

    private static PlatformInvoiceDto ToPlatformInvoiceDto(Invoice invoice, IReadOnlyDictionary<Guid, string> tenantNames)
    {
        return new PlatformInvoiceDto
        {
            Id = invoice.Id,
            TenantId = invoice.TenantId,
            TenantName = ResolveTenantName(invoice.TenantId, tenantNames),
            Amount = invoice.Amount,
            Status = invoice.Status,
            DueDate = invoice.DueDate,
            BillingPeriodStart = invoice.BillingPeriodStart
        };
    }

    /// <summary>"Host" for host-side rows; "Deleted workspace" when the tenant no longer exists.</summary>
    private static string ResolveTenantName(Guid? tenantId, IReadOnlyDictionary<Guid, string> tenantNames)
    {
        if (!tenantId.HasValue)
        {
            return HostName;
        }

        return tenantNames.TryGetValue(tenantId.Value, out var name) ? name : DeletedWorkspaceName;
    }

    /// <summary>Names of the existing (not deleted) workspaces, by tenant id.</summary>
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
