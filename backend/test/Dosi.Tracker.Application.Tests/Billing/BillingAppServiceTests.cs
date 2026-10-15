using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Billing;

public abstract class BillingAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IBillingAppService _billingAppService;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected BillingAppServiceTests()
    {
        _billingAppService = GetRequiredService<IBillingAppService>();
        _invoiceRepository = GetRequiredService<IRepository<Invoice, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    [Fact]
    public async Task GetInvoices_Should_Page_And_Order_By_DueDate_Descending()
    {
        var baseDate = new DateTime(2026, 7, 1, 0, 0, 0, DateTimeKind.Utc);
        await WithUnitOfWorkAsync(async () =>
        {
            await _invoiceRepository.InsertAsync(new Invoice(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, 10m, baseDate));
            await _invoiceRepository.InsertAsync(new Invoice(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, 30m, baseDate.AddMonths(2)));
            await _invoiceRepository.InsertAsync(new Invoice(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, 20m, baseDate.AddMonths(1)));
        });

        var page = await _billingAppService.GetInvoicesAsync(new PagedAndSortedResultRequestDto
        {
            SkipCount = 0,
            MaxResultCount = 2
        });

        page.TotalCount.ShouldBe(3);
        page.Items.Count.ShouldBe(2);
        page.Items[0].Amount.ShouldBe(30m); // newest due date first
        page.Items[1].Amount.ShouldBe(20m);
        page.Items[0].Status.ShouldBe("pending");
    }

    [Fact]
    public async Task GetInvoices_Should_Show_The_Billed_Period()
    {
        var period = new DateTime(2026, 9, 1, 0, 0, 0, DateTimeKind.Utc);
        await WithUnitOfWorkAsync(async () =>
        {
            await _invoiceRepository.InsertAsync(
                new Invoice(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, 18m, period.AddDays(14)) { BillingPeriodStart = period });
            await _invoiceRepository.InsertAsync(
                new Invoice(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, 12m, period.AddMonths(-2))); // from before billing periods
        });

        var page = await _billingAppService.GetInvoicesAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 10 });

        page.Items[0].BillingPeriodStart.ShouldBe(period);
        page.Items[1].BillingPeriodStart.ShouldBeNull();
    }
}
