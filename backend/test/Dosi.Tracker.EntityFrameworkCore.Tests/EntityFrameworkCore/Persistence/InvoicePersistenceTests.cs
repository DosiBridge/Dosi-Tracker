using System;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class InvoicePersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly DateTime October = new(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc);

    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly ICurrentTenant _currentTenant;

    public InvoicePersistenceTests()
    {
        _invoiceRepository = GetRequiredService<IRepository<Invoice, Guid>>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    private async Task<Invoice> InsertAsync(Guid tenantId, DateTime? billingPeriodStart)
    {
        var invoice = new Invoice(Guid.NewGuid(), tenantId, 12m, October.AddDays(14))
        {
            BillingPeriodStart = billingPeriodStart
        };

        await WithUnitOfWorkAsync(() => _invoiceRepository.InsertAsync(invoice, autoSave: true));
        return invoice;
    }

    [Fact]
    public async Task A_Workspace_Should_Have_At_Most_One_Invoice_Per_Billing_Period()
    {
        var tenantId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var invoice = await InsertAsync(tenantId, October);
            (await WithUnitOfWorkAsync(() => _invoiceRepository.GetAsync(invoice.Id))).BillingPeriodStart.ShouldBe(October);

            // Concurrent sweeps on two replicas: the second insert for the same month is refused by the database.
            await Should.ThrowAsync<DbUpdateException>(() => InsertAsync(tenantId, October));

            await InsertAsync(tenantId, October.AddMonths(1)); // the next month is a new period
            (await WithUnitOfWorkAsync(() => _invoiceRepository.GetCountAsync())).ShouldBe(2);
        }
    }

    [Fact]
    public async Task Legacy_Invoices_Without_A_Period_Should_Never_Collide()
    {
        var tenantId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            await InsertAsync(tenantId, null);
            await InsertAsync(tenantId, null);

            (await WithUnitOfWorkAsync(() => _invoiceRepository.GetCountAsync())).ShouldBe(2);
        }
    }

    [Fact]
    public async Task A_Deleted_Invoice_Should_Not_Block_A_New_One_For_The_Same_Period()
    {
        var tenantId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var voided = await InsertAsync(tenantId, October);
            await WithUnitOfWorkAsync(() => _invoiceRepository.DeleteAsync(voided.Id)); // soft delete

            await InsertAsync(tenantId, October);

            (await WithUnitOfWorkAsync(() => _invoiceRepository.GetCountAsync())).ShouldBe(1);
        }
    }

    [Fact]
    public async Task Different_Workspaces_Can_Be_Invoiced_For_The_Same_Period()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();

        using (_currentTenant.Change(tenantA))
        {
            await InsertAsync(tenantA, October);
        }

        using (_currentTenant.Change(tenantB))
        {
            await InsertAsync(tenantB, October);
            (await WithUnitOfWorkAsync(() => _invoiceRepository.GetCountAsync())).ShouldBe(1);
        }
    }
}
