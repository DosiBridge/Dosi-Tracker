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
using Dosi.Tracker.Workspaces;
using Volo.Abp.Validation;

namespace Dosi.Tracker.Platform;

/// <summary>Platform console. Requires <see cref="TrackerPermissions.Platform.Default"/>, which only the global
/// platform-admin role holds; it works across every workspace (the workspace data filter is disabled).</summary>
[Authorize(TrackerPermissions.Platform.Default)]
[WorkspaceNotRequired]
public class PlatformAppService : TrackerAppService, IPlatformAppService
{
    /// <summary>Workspace name shown for rows of a workspace that no longer exists.</summary>
    public const string DeletedWorkspaceName = "Deleted workspace";

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IGuidGenerator _guidGenerator;
    private readonly SeatCounter _seatCounter;
    private readonly SubscriptionManager _subscriptionManager;

    public PlatformAppService(
        IRepository<Workspace, Guid> workspaceRepository,
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Invoice, Guid> invoiceRepository,
        IIdentityUserRepository userRepository,
        IDataFilter dataFilter,
        IGuidGenerator guidGenerator,
        SeatCounter seatCounter,
        SubscriptionManager subscriptionManager)
    {
        _workspaceRepository = workspaceRepository;
        _workspaceMemberRepository = workspaceMemberRepository;
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
        var workspaceNames = await GetWorkspaceNamesAsync();
        var plans = await _planRepository.GetListAsync();

        // Platform aggregation: every workspace's subscriptions and invoices.
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            // Customers only: a deleted (or unknown) workspace's subscription is neither revenue nor a subscriber.
            var subscriptions = (await _subscriptionRepository.GetListAsync())
                .Where(s => workspaceNames.ContainsKey(s.WorkspaceId))
                .ToList();
            var invoices = await _invoiceRepository.GetListAsync();

            // Occupied (billable) seats per workspace: the same count the invoice and the seat limit use.
            var seatsByWorkspace = await _seatCounter.CountByWorkspaceAsync();

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
                var seats = Math.Max(1, seatsByWorkspace.GetValueOrDefault(sub.WorkspaceId));
                mrr += plan.PricePerUser * seats;
                paidSeats += seats;
            }

            return new PlatformOverviewDto
            {
                WorkspaceCount = workspaceNames.Count,
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
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            // Count and page in the database; only the requested page is materialized.
            var query = await _invoiceRepository.GetQueryableAsync();
            var totalCount = await AsyncExecuter.CountAsync(query);
            var page = await AsyncExecuter.ToListAsync(query
                .OrderByDescending(i => i.DueDate)
                .ThenBy(i => i.Id) // stable order across pages
                .PageBy(input.SkipCount, input.MaxResultCount));

            var workspaceNames = await GetWorkspaceNamesAsync();

            var items = page
                .Select(i => ToPlatformInvoiceDto(i, workspaceNames))
                .ToList();

            return new PagedResultDto<PlatformInvoiceDto>(totalCount, items);
        }
    }

    public async Task<PagedResultDto<PlatformUserDto>> GetUsersAsync(PagedAndSortedResultRequestDto input)
    {
        var workspaceNames = await GetWorkspaceNamesAsync();
        var count = await _userRepository.GetCountAsync();
        var users = await _userRepository.GetListAsync(
            sorting: nameof(IdentityUser.UserName),
            maxResultCount: input.MaxResultCount,
            skipCount: input.SkipCount);

        var userIds = users.Select(u => u.Id).ToList();
        var memberships = (await _workspaceMemberRepository.GetListAsync(m =>
                userIds.Contains(m.UserId) && m.Status == WorkspaceMemberStatus.Active))
            .GroupBy(m => m.UserId)
            .ToDictionary(g => g.Key, g => g.Select(m => m.WorkspaceId).ToList());

        var items = users.Select(u => new PlatformUserDto
        {
            Id = u.Id,
            UserName = u.UserName,
            Email = u.Email,
            Name = u.Name,
            IsActive = u.IsActive,
            WorkspaceNames = (memberships.GetValueOrDefault(u.Id) ?? new List<Guid>())
                .Where(workspaceNames.ContainsKey)
                .Select(id => workspaceNames[id])
                .OrderBy(n => n, StringComparer.OrdinalIgnoreCase)
                .ToList()
        }).ToList();

        return new PagedResultDto<PlatformUserDto>(count, items);
    }

    public async Task<ListResultDto<PlanDto>> GetPlansAsync()
    {
        var plans = await _planRepository.GetListAsync();
        return new ListResultDto<PlanDto>(
            ObjectMapper.Map<List<Plan>, List<PlanDto>>(plans.OrderBy(p => p.PricePerUser).ToList()));
    }

    public async Task<PlanDto> CreatePlanAsync(CreateUpdatePlanDto input)
    {
        var name = input.Name.Trim();
        await CheckPlanNameIsFreeAsync(name, exceptPlanId: null);

        var plan = new Plan(_guidGenerator.Create(), name, input.PricePerUser, input.MaxSeats, input.TrialDays);
        await _planRepository.InsertAsync(plan, autoSave: true);
        return ObjectMapper.Map<Plan, PlanDto>(plan);
    }

    public async Task<PlanDto> UpdatePlanAsync(Guid id, CreateUpdatePlanDto input)
    {
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
        // A plan still referenced by any workspace's subscription cannot be removed (it would orphan
        // seat limits, billing and the overview's plan counts). Check across every existing workspace.
        var workspaceNames = await GetWorkspaceNamesAsync();
        List<Subscription> subscribers;
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            subscribers = await _subscriptionRepository.GetListAsync(s => s.PlanId == id);
        }

        var inUse = subscribers.Count(s => workspaceNames.ContainsKey(s.WorkspaceId));
        if (inUse > 0)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanInUse)
                .WithData("planId", id)
                .WithData("subscriptions", inUse);
        }

        await _planRepository.DeleteAsync(id);
    }

    public async Task<ListResultDto<WorkspaceSubscriptionDto>> GetWorkspaceSubscriptionsAsync()
    {
        var workspaces = await _workspaceRepository.GetListAsync();
        var plans = (await _planRepository.GetListAsync()).ToDictionary(p => p.Id);

        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            var subscriptions = (await _subscriptionRepository.GetListAsync())
                .GroupBy(s => s.WorkspaceId)
                .ToDictionary(g => g.Key, g => g.OrderByDescending(s => s.CreationTime).First());
            var seatsByWorkspace = await _seatCounter.CountByWorkspaceAsync();

            var items = workspaces
                .OrderBy(w => w.Name)
                .Select(w =>
                {
                    var subscription = subscriptions.GetValueOrDefault(w.Id);
                    var plan = subscription == null ? null : plans.GetValueOrDefault(subscription.PlanId);
                    return new WorkspaceSubscriptionDto
                    {
                        WorkspaceId = w.Id,
                        WorkspaceName = w.Name,
                        SubscriptionId = subscription?.Id,
                        PlanId = subscription?.PlanId,
                        PlanName = plan?.Name,
                        Status = subscription?.Status,
                        TrialEndsAt = subscription?.TrialEndsAt,
                        SeatsUsed = seatsByWorkspace.GetValueOrDefault(w.Id),
                        MaxSeats = plan?.MaxSeats
                    };
                })
                .ToList();

            return new ListResultDto<WorkspaceSubscriptionDto>(items);
        }
    }

    public async Task<SubscriptionDto> ChangeWorkspacePlanAsync(Guid workspaceId, Guid planId)
    {
        await _workspaceRepository.GetAsync(workspaceId);

        var plan = await _planRepository.FindAsync(planId);
        if (plan == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                .WithData("planId", planId);
        }

        using (CurrentWorkspace.Change(workspaceId))
        {
            var subscription = await _subscriptionManager.ChangePlanAsync(plan);
            return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
        }
    }

    /// <inheritdoc />
    /// <remarks><paramref name="workspaceId"/> is optional only so that ABP keeps it in the query string.</remarks>
    public async Task<SubscriptionDto> ActivateSubscriptionAsync([Optional] Guid workspaceId)
    {
        await _workspaceRepository.GetAsync(workspaceId);

        using (CurrentWorkspace.Change(workspaceId))
        {
            var subscription = await _subscriptionManager.ActivateAsync();
            return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
        }
    }

    /// <inheritdoc />
    /// <remarks><paramref name="workspaceId"/> is optional only so that ABP keeps it in the query string.</remarks>
    public async Task<SubscriptionDto> SuspendSubscriptionAsync([Optional] Guid workspaceId)
    {
        await _workspaceRepository.GetAsync(workspaceId);

        using (CurrentWorkspace.Change(workspaceId))
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
        Invoice invoice;
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            invoice = await _invoiceRepository.GetAsync(invoiceId);
        }

        using (CurrentWorkspace.Change(invoice.WorkspaceId))
        {
            await _subscriptionManager.MarkInvoicePaidAsync(invoice);
        }

        return ToPlatformInvoiceDto(invoice, await GetWorkspaceNamesAsync());
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

    private static PlatformInvoiceDto ToPlatformInvoiceDto(Invoice invoice, IReadOnlyDictionary<Guid, string> workspaceNames)
    {
        return new PlatformInvoiceDto
        {
            Id = invoice.Id,
            WorkspaceId = invoice.WorkspaceId,
            WorkspaceName = workspaceNames.TryGetValue(invoice.WorkspaceId, out var name) ? name : DeletedWorkspaceName,
            Amount = invoice.Amount,
            Status = invoice.Status,
            DueDate = invoice.DueDate,
            BillingPeriodStart = invoice.BillingPeriodStart
        };
    }

    private async Task<Dictionary<Guid, string>> GetWorkspaceNamesAsync()
    {
        return (await _workspaceRepository.GetListAsync()).ToDictionary(w => w.Id, w => w.Name);
    }
}
