using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
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

    private Task<Guid> CreateProjectAsync(string title)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var project = await _projectRepository.InsertAsync(
                new Project(_guidGenerator.Create(), null, title));
            return project.Id;
        });
    }

    [Fact]
    public async Task Should_Invoice_PricePerUser_Times_Seats_Once_Per_Month()
    {
        var starter = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Starter")); // $6/user

        // Two distinct users occupy seats (one of them on two projects — still one seat).
        var userA = Guid.NewGuid();
        var userB = Guid.NewGuid();
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
}
