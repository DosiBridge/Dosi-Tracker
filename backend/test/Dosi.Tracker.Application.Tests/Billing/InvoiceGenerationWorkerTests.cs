using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.Billing;

public abstract class InvoiceGenerationWorkerTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;
    private readonly IDataFilter _dataFilter;

    protected InvoiceGenerationWorkerTests()
    {
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _dataFilter = GetRequiredService<IDataFilter>();
    }

    private async Task SubscribeAsync(Guid tenantId, Guid planId, string status)
    {
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(
                new Subscription(_guidGenerator.Create(), tenantId, planId) { Status = status }));
        }
    }

    private Task<List<Invoice>> AllInvoicesAsync()
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (_dataFilter.Disable<IMultiTenant>())
            {
                return await _invoices.GetListAsync();
            }
        });
    }

    [Fact]
    public async Task Sweep_Should_Invoice_Active_Subscriptions_But_Never_Trialing_Ones()
    {
        var starter = await WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter")); // $6/user
        var activeTenant = Guid.NewGuid();
        var trialingTenant = Guid.NewGuid();
        var pastDueTenant = Guid.NewGuid();

        await SubscribeAsync(activeTenant, starter.Id, "active");
        await SubscribeAsync(trialingTenant, starter.Id, "trialing");
        await SubscribeAsync(pastDueTenant, starter.Id, "past_due");

        var worker = GetRequiredService<InvoiceGenerationWorker>();
        await worker.GenerateInvoicesAsync(ServiceProvider);

        var invoices = await AllInvoicesAsync();
        var invoice = invoices.ShouldHaveSingleItem();
        invoice.TenantId.ShouldBe(activeTenant);
        invoice.Amount.ShouldBe(6m); // one seat minimum

        // A second sweep in the same month bills nobody again.
        await worker.GenerateInvoicesAsync(ServiceProvider);
        (await AllInvoicesAsync()).Count.ShouldBe(1);
    }
}
