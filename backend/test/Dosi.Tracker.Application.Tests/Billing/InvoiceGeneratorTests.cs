using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Billing;

public abstract class InvoiceGeneratorTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly InvoiceGenerator _invoiceGenerator;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected InvoiceGeneratorTests()
    {
        _invoiceGenerator = GetRequiredService<InvoiceGenerator>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _invoiceRepository = GetRequiredService<IRepository<Invoice, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private Task<Guid> CreateProjectAsync(string title)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var project = await _projectRepository.InsertAsync(
                new Project(_guidGenerator.Create(), null, title));
            return project.Id;
        });
    }

    /// <summary>A person with an account in the (host) workspace: only they can occupy a seat.</summary>
    private Task<Guid> CreateUserAsync()
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var userName = $"member-{Guid.NewGuid():N}";
            var user = new IdentityUser(_guidGenerator.Create(), userName, $"{userName}@billing.test");
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
            return user.Id;
        });
    }

    [Fact]
    public async Task Should_Invoice_PricePerUser_Times_Seats_Once_Per_Month()
    {
        var starter = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Starter")); // $6/user

        // Two distinct users occupy seats (one of them on two projects — still one seat).
        var userA = await CreateUserAsync();
        var userB = await CreateUserAsync();
        var projectId = await CreateProjectAsync("Billable A");
        var secondProjectId = await CreateProjectAsync("Billable B");
        await WithUnitOfWorkAsync(async () =>
        {
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, projectId, userA));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, secondProjectId, userA));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, projectId, userB));
        });

        var subscription = new Subscription(_guidGenerator.Create(), null, starter.Id);

        var invoice = await WithUnitOfWorkAsync(
            () => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription));

        invoice.ShouldNotBeNull();
        invoice.Amount.ShouldBe(12m); // $6 × 2 seats
        invoice.Status.ShouldBe("pending");

        // Second sweep in the same month must not double-bill.
        var second = await WithUnitOfWorkAsync(
            () => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription));
        second.ShouldBeNull();

        await WithUnitOfWorkAsync(async () =>
        {
            (await _invoiceRepository.GetCountAsync()).ShouldBe(1);
        });
    }

    [Fact]
    public async Task Should_Not_Invoice_Free_Plans()
    {
        var free = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Free"));

        var subscription = new Subscription(_guidGenerator.Create(), null, free.Id);

        var invoice = await WithUnitOfWorkAsync(
            () => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription));

        invoice.ShouldBeNull();
    }

    [Fact]
    public async Task Should_Bill_At_Least_One_Seat()
    {
        var business = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Business")); // $12/user

        // No project members at all — the workspace admin still occupies a seat.
        var subscription = new Subscription(_guidGenerator.Create(), null, business.Id);

        var invoice = await WithUnitOfWorkAsync(
            () => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription));

        invoice.ShouldNotBeNull();
        invoice.Amount.ShouldBe(12m);
    }

    [Fact]
    public async Task The_Invoice_Should_Record_Its_Billing_Period_And_Payment_Term()
    {
        var starter = await WithUnitOfWorkAsync(() => _planRepository.FirstAsync(p => p.Name == "Starter"));
        var subscription = new Subscription(_guidGenerator.Create(), null, starter.Id);

        var invoice = await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription));

        invoice.ShouldNotBeNull();
        invoice.BillingPeriodStart.ShouldBe(Invoice.GetBillingPeriodStart(DateTime.UtcNow));
        invoice.DueDate.ShouldBeInRange(
            DateTime.UtcNow.AddDays(InvoiceGenerator.PaymentTermDays).AddMinutes(-1),
            DateTime.UtcNow.AddDays(InvoiceGenerator.PaymentTermDays).AddMinutes(1));
    }

    [Fact]
    public async Task Each_Billing_Period_Should_Be_Invoiced_Once()
    {
        var starter = await WithUnitOfWorkAsync(() => _planRepository.FirstAsync(p => p.Name == "Starter"));
        var subscription = new Subscription(_guidGenerator.Create(), null, starter.Id);
        var thisMonth = Invoice.GetBillingPeriodStart(DateTime.UtcNow);
        var lastMonth = thisMonth.AddMonths(-1);

        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForPeriodAsync(subscription, lastMonth.AddDays(20))))
            .ShouldNotBeNull().BillingPeriodStart.ShouldBe(lastMonth);
        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForPeriodAsync(subscription, thisMonth)))
            .ShouldNotBeNull().BillingPeriodStart.ShouldBe(thisMonth);

        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForPeriodAsync(subscription, lastMonth))).ShouldBeNull();
        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription))).ShouldBeNull();
        (await WithUnitOfWorkAsync(() => _invoiceRepository.GetCountAsync())).ShouldBe(2);
    }

    [Fact]
    public async Task A_Legacy_Invoice_Of_The_Month_Should_Count_As_That_Months_Invoice()
    {
        // Invoices from before billing periods have none: their creation month stands in for it.
        var starter = await WithUnitOfWorkAsync(() => _planRepository.FirstAsync(p => p.Name == "Starter"));
        var subscription = new Subscription(_guidGenerator.Create(), null, starter.Id);
        await WithUnitOfWorkAsync(() => _invoiceRepository.InsertAsync(
            new Invoice(_guidGenerator.Create(), null, 6m, DateTime.UtcNow.AddDays(14))));

        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForCurrentMonthAsync(subscription))).ShouldBeNull();

        // ...but not for another month.
        var lastMonth = Invoice.GetBillingPeriodStart(DateTime.UtcNow).AddMonths(-1);
        (await WithUnitOfWorkAsync(() => _invoiceGenerator.GenerateForPeriodAsync(subscription, lastMonth))).ShouldNotBeNull();
    }

    [Fact]
    public async Task Seats_On_Deleted_Projects_Should_Not_Be_Invoiced()
    {
        var starter = await WithUnitOfWorkAsync(() => _planRepository.FirstAsync(p => p.Name == "Starter")); // $6/user
        var kept = await CreateProjectAsync("Kept");
        var deleted = await CreateProjectAsync("Deleted");
        var stays = await CreateUserAsync();
        var onDeletedProjectOnly = new[] { await CreateUserAsync(), await CreateUserAsync(), await CreateUserAsync() };
        await WithUnitOfWorkAsync(async () =>
        {
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, kept, stays));
            foreach (var userId in onDeletedProjectOnly)
            {
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, deleted, userId));
            }
        });
        await WithUnitOfWorkAsync(() => _projectRepository.DeleteAsync(deleted));

        var invoice = await WithUnitOfWorkAsync(() =>
            _invoiceGenerator.GenerateForCurrentMonthAsync(new Subscription(_guidGenerator.Create(), null, starter.Id)));

        invoice.ShouldNotBeNull().Amount.ShouldBe(6m);
        (await WithUnitOfWorkAsync(() => _invoiceGenerator.CountSeatsAsync())).ShouldBe(1);
    }
}
