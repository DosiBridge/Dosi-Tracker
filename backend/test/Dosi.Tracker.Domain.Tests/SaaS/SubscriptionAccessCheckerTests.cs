using System;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class SubscriptionAccessCheckerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly ISubscriptionAccessChecker _checker;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IGuidGenerator _guidGenerator;

    protected SubscriptionAccessCheckerTests()
    {
        _checker = GetRequiredService<ISubscriptionAccessChecker>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _invoiceRepository = GetRequiredService<IRepository<Invoice, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private async Task SubscribeAsync(Guid tenantId, string status, DateTime? trialEndsAt = null)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _subscriptionRepository.InsertAsync(
                new Subscription(_guidGenerator.Create(), tenantId, Guid.NewGuid())
                {
                    Status = status,
                    TrialEndsAt = trialEndsAt
                }));
        }
    }

    private async Task InvoiceAsync(Guid tenantId, DateTime dueDate, string status = Invoice.PendingStatus, bool excused = false)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            var invoice = new Invoice(_guidGenerator.Create(), tenantId, 12m, dueDate) { Status = status };
            if (excused)
            {
                invoice.ExcuseOverdue();
            }

            await WithUnitOfWorkAsync(() => _invoiceRepository.InsertAsync(invoice));
        }
    }

    /// <summary>Asks the checker from inside the given workspace (null = the host).</summary>
    private async Task<bool> IsUsableAsync(Guid? tenantId)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(() => _checker.IsUsableAsync());
        }
    }

    /// <summary>A fresh workspace with one subscription in the given state.</summary>
    private async Task<bool> IsUsableWithAsync(string status, DateTime? trialEndsAt = null)
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, status, trialEndsAt);
        return await IsUsableAsync(tenantId);
    }

    [Fact]
    public async Task The_Host_Should_Always_Be_Usable()
    {
        (await IsUsableAsync(null)).ShouldBeTrue();
    }

    [Fact]
    public async Task A_Workspace_Without_A_Subscription_Should_Be_Usable()
    {
        (await IsUsableAsync(Guid.NewGuid())).ShouldBeTrue();
    }

    [Fact]
    public async Task An_Active_Subscription_Should_Be_Usable()
    {
        (await IsUsableWithAsync(Subscription.ActiveStatus)).ShouldBeTrue();
    }

    [Fact]
    public async Task A_Running_Trial_Should_Be_Usable()
    {
        (await IsUsableWithAsync(Subscription.TrialingStatus, DateTime.UtcNow.AddDays(3))).ShouldBeTrue();
    }

    [Fact]
    public async Task A_Trial_Without_An_End_Date_Should_Be_Usable()
    {
        (await IsUsableWithAsync(Subscription.TrialingStatus, trialEndsAt: null)).ShouldBeTrue();
    }

    [Fact]
    public async Task An_Expired_Trial_Should_Not_Be_Usable()
    {
        // Not yet swept to past_due by TrialExpiryWorker: the end date alone decides.
        (await IsUsableWithAsync(Subscription.TrialingStatus, DateTime.UtcNow.AddMinutes(-1))).ShouldBeFalse();
    }

    [Theory]
    [InlineData(Subscription.CanceledStatus)]
    [InlineData(Subscription.ExpiredStatus)]
    [InlineData(Subscription.SuspendedStatus)]
    [InlineData("")]
    [InlineData("unknown")]
    public async Task Ending_And_Unknown_Statuses_Should_Not_Be_Usable(string status)
    {
        (await IsUsableWithAsync(status)).ShouldBeFalse();
    }

    [Fact]
    public async Task Past_Due_Should_Be_Usable_During_The_Grace_Period_After_The_Oldest_Unpaid_Invoice()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, Subscription.PastDueStatus);
        // The oldest unpaid invoice fell due 6 days ago: one day of grace left.
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-(Subscription.PastDueGraceDays - 1)));
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-1));

        (await IsUsableAsync(tenantId)).ShouldBeTrue();
    }

    [Fact]
    public async Task Past_Due_Should_Not_Be_Usable_After_The_Grace_Period()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, Subscription.PastDueStatus);
        // A newer invoice does not extend the grace: the oldest unpaid one fell due 8 days ago.
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-(Subscription.PastDueGraceDays + 1)));
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-1));

        (await IsUsableAsync(tenantId)).ShouldBeFalse();
    }

    [Fact]
    public async Task Paid_Invoices_Should_Not_Count_Toward_The_Grace_Period()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, Subscription.PastDueStatus);
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-30), Invoice.PaidStatus);
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-2), Invoice.FailedStatus);

        (await IsUsableAsync(tenantId)).ShouldBeTrue(); // the failed (unpaid) one is the oldest unpaid: 5 days left
    }

    [Fact]
    public async Task Invoices_The_Host_Excused_Should_Not_Count_Toward_The_Grace_Period()
    {
        // The host reactivated the workspace over an invoice that is still unpaid; a later one went unpaid too.
        var tenantId = Guid.NewGuid();
        var laterDueDate = DateTime.UtcNow.AddDays(-2);
        await SubscribeAsync(tenantId, Subscription.PastDueStatus);
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-40), excused: true);
        await InvoiceAsync(tenantId, laterDueDate);

        (await IsUsableAsync(tenantId)).ShouldBeTrue(); // the grace runs from the later invoice: 5 days left

        using (_currentWorkspace.Change(tenantId))
        {
            (await WithUnitOfWorkAsync(() => GetRequiredService<SubscriptionAccessChecker>().GetPastDueGraceEndAsync()))
                .ShouldBe(laterDueDate.AddDays(Subscription.PastDueGraceDays));
        }
    }

    [Fact]
    public async Task Past_Due_With_Only_Excused_Invoices_Should_Have_No_Grace()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, Subscription.PastDueStatus);
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-1), excused: true);

        (await IsUsableAsync(tenantId)).ShouldBeFalse();
    }

    [Fact]
    public async Task Past_Due_Without_An_Unpaid_Invoice_Should_Not_Be_Usable()
    {
        // An ended trial: nothing was ever invoiced, so there is no grace period.
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, Subscription.PastDueStatus, DateTime.UtcNow.AddDays(-1));

        (await IsUsableAsync(tenantId)).ShouldBeFalse();
    }

    [Fact]
    public async Task Another_Workspaces_Invoices_Should_Not_Grant_Grace()
    {
        var pastDueTenant = Guid.NewGuid();
        var otherTenant = Guid.NewGuid();
        await SubscribeAsync(pastDueTenant, Subscription.PastDueStatus);
        await InvoiceAsync(otherTenant, DateTime.UtcNow.AddDays(-1));

        (await IsUsableAsync(pastDueTenant)).ShouldBeFalse();
    }

    [Fact]
    public async Task Only_The_Current_Workspaces_Subscription_Should_Count()
    {
        var pastDueTenant = Guid.NewGuid();
        var activeTenant = Guid.NewGuid();
        await SubscribeAsync(pastDueTenant, Subscription.PastDueStatus);
        await SubscribeAsync(activeTenant, Subscription.ActiveStatus);

        (await IsUsableAsync(pastDueTenant)).ShouldBeFalse();
        (await IsUsableAsync(activeTenant)).ShouldBeTrue();
        (await IsUsableAsync(null)).ShouldBeTrue();
    }
}
