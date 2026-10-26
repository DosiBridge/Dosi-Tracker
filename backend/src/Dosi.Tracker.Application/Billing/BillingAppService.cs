using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Billing;

[Authorize(TrackerPermissions.Billing.Default)]
public class BillingAppService : TrackerAppService, IBillingAppService
{
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly SubscriptionManager _subscriptionManager;
    private readonly SeatCounter _seatCounter;

    public BillingAppService(
        IRepository<Invoice, Guid> invoiceRepository,
        IRepository<Plan, Guid> planRepository,
        SubscriptionManager subscriptionManager,
        SeatCounter seatCounter)
    {
        _invoiceRepository = invoiceRepository;
        _planRepository = planRepository;
        _subscriptionManager = subscriptionManager;
        _seatCounter = seatCounter;
    }

    public async Task<PagedResultDto<InvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input)
    {
        var query = await _invoiceRepository.GetQueryableAsync();
        var totalCount = await AsyncExecuter.CountAsync(query);
        var items = await AsyncExecuter.ToListAsync(
            query
                .OrderByDescending(i => i.DueDate) // Deterministic order; Skip/Take without OrderBy is undefined.
                .PageBy(input.SkipCount, input.MaxResultCount));

        return new PagedResultDto<InvoiceDto>(
            totalCount,
            ObjectMapper.Map<List<Invoice>, List<InvoiceDto>>(items)
        );
    }

    public async Task<SubscriptionDto> ChangePlanAsync(ChangePlanDto input)
    {
        var plan = await _planRepository.FindAsync(input.PlanId)
                   ?? throw new Volo.Abp.BusinessException(TrackerDomainErrorCodes.PlanNotFound).WithData("planId", input.PlanId);

        // Under the seat lock: nobody may take a seat between the check that the team fits the plan and the change.
        await using var seatLock = await _seatCounter.LockAsync();
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        var subscription = await _subscriptionManager.ChangePlanByOwnerAsync(plan);
        await uow.CompleteAsync();

        return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
    }

    public async Task<SubscriptionDto> CancelAsync()
    {
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        var subscription = await _subscriptionManager.CancelByOwnerAsync();
        await uow.CompleteAsync();

        return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
    }
}
