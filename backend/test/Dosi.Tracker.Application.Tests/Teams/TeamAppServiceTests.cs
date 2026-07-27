using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Teams;

public abstract class TeamAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly ITeamAppService _teamAppService;
    private readonly IProjectAppService _projectAppService;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected TeamAppServiceTests()
    {
        _teamAppService = GetRequiredService<ITeamAppService>();
        _projectAppService = GetRequiredService<IProjectAppService>();
        _notificationRepository = GetRequiredService<IRepository<Notification, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private async Task<Guid> CreateProjectAsync()
    {
        var project = await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "Team Test Project",
            Color = "#006bff",
            IntervalMinutes = 10
        });
        return project.Id;
    }

    [Fact]
    public async Task Should_Add_And_List_Project_Members()
    {
        var projectId = await CreateProjectAsync();
        var userId = Guid.NewGuid();

        var member = await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = userId,
            Role = "Worker"
        });

        member.ProjectId.ShouldBe(projectId);
        member.UserId.ShouldBe(userId);
        member.Role.ShouldBe("Worker");

        var members = await _teamAppService.GetProjectMembersAsync(projectId);

        // Creator is auto-enrolled as Admin, plus the member just added.
        members.Items.Count.ShouldBe(2);
        members.Items.ShouldContain(m => m.UserId == CurrentUserId && m.Role == "Admin");
        members.Items.ShouldContain(m => m.UserId == userId && m.Role == "Worker");
    }

    [Fact]
    public async Task AddMember_Should_Reject_Duplicate_Membership()
    {
        var projectId = await CreateProjectAsync();
        var userId = Guid.NewGuid();

        await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = userId,
            Role = "Worker"
        });

        var exception = await Should.ThrowAsync<BusinessException>(
            () => _teamAppService.AddMemberAsync(new CreateProjectMemberDto
            {
                ProjectId = projectId,
                UserId = userId,
                Role = "Admin"
            }));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.DuplicateProjectMember);
    }

    [Fact]
    public async Task AddMember_Should_Reject_Unknown_Project()
    {
        await Should.ThrowAsync<EntityNotFoundException>(
            () => _teamAppService.AddMemberAsync(new CreateProjectMemberDto
            {
                ProjectId = Guid.NewGuid(),
                UserId = Guid.NewGuid(),
                Role = "Worker"
            }));
    }

    [Fact]
    public async Task RemoveMember_Should_Remove_The_Membership()
    {
        var projectId = await CreateProjectAsync();

        var member = await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = Guid.NewGuid(),
            Role = "Worker"
        });

        await _teamAppService.RemoveMemberAsync(member.Id);

        var members = await _teamAppService.GetProjectMembersAsync(projectId);
        members.Items.Count.ShouldBe(1); // only the auto-enrolled creator remains
        members.Items[0].UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task AddMember_Should_Notify_The_New_Member()
    {
        var projectId = await CreateProjectAsync();
        var userId = Guid.NewGuid();

        await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = userId,
            Role = "Worker"
        });

        await WithUnitOfWorkAsync(async () =>
        {
            var notifications = await _notificationRepository.GetListAsync(n => n.UserId == userId);
            notifications.Count.ShouldBe(1);
            notifications[0].Message.ShouldContain("Team Test Project");
            notifications[0].IsRead.ShouldBeFalse();
        });
    }

    [Fact]
    public async Task AddMember_Should_Persist_The_Hourly_Rate()
    {
        var projectId = await CreateProjectAsync();

        var member = await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = Guid.NewGuid(),
            Role = "Worker",
            HourlyRate = 42.5m
        });

        member.HourlyRate.ShouldBe(42.5m);

        var members = await _teamAppService.GetProjectMembersAsync(projectId);
        members.Items.First(m => m.Id == member.Id).HourlyRate.ShouldBe(42.5m);
    }

    [Fact]
    public async Task Invite_Should_Create_A_New_User_With_A_OneTime_Password()
    {
        var projectId = await CreateProjectAsync();

        var result = await _teamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "new.worker@acme.test",
            Role = "Worker",
            HourlyRate = 30m
        });

        result.UserCreated.ShouldBeTrue();
        result.InitialPassword.ShouldNotBeNullOrWhiteSpace();
        result.Member.Role.ShouldBe("Worker");
        result.Member.HourlyRate.ShouldBe(30m);

        // The invitee is a member and was notified.
        var members = await _teamAppService.GetProjectMembersAsync(projectId);
        members.Items.ShouldContain(m => m.UserId == result.Member.UserId);

        await WithUnitOfWorkAsync(async () =>
        {
            var notifications = await _notificationRepository.GetListAsync(
                n => n.UserId == result.Member.UserId);
            notifications.Count.ShouldBe(1);
        });
    }

    [Fact]
    public async Task Invite_Should_Reuse_An_Existing_User_Without_A_Password()
    {
        var projectA = await CreateProjectAsync();

        var first = await _teamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectA,
            Email = "repeat.worker@acme.test",
            Role = "Worker"
        });

        var projectB = (await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "Second Invite Project", Color = "#112233", IntervalMinutes = 10
        })).Id;

        var second = await _teamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectB,
            Email = "repeat.worker@acme.test",
            Role = "Admin"
        });

        second.UserCreated.ShouldBeFalse();
        second.InitialPassword.ShouldBeNull();
        second.Member.UserId.ShouldBe(first.Member.UserId); // same identity, new membership
    }

    [Fact]
    public async Task AddMember_Should_Enforce_The_Plan_Seat_Limit()
    {
        // Free plan (seeded): 3 seats. Creator occupies the first seat automatically.
        var freePlan = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Free"));
        await WithUnitOfWorkAsync(async () =>
        {
            await _subscriptionRepository.InsertAsync(
                new Subscription(_guidGenerator.Create(), null, freePlan.Id));
        });

        var projectId = await CreateProjectAsync();

        await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId, UserId = Guid.NewGuid(), Role = "Worker"
        });
        await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId, UserId = Guid.NewGuid(), Role = "Worker"
        });

        // Seat 4 must be rejected on the 3-seat plan.
        var exception = await Should.ThrowAsync<BusinessException>(
            () => _teamAppService.AddMemberAsync(new CreateProjectMemberDto
            {
                ProjectId = projectId, UserId = Guid.NewGuid(), Role = "Worker"
            }));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);

        // But an existing member (already occupying a seat) can join another project.
        var secondProject = await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "Second Project", Color = "#112233", IntervalMinutes = 10
        });
        var existingMemberId = (await _teamAppService.GetProjectMembersAsync(projectId))
            .Items.First(m => m.UserId != CurrentUserId).UserId;

        var again = await _teamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = secondProject.Id, UserId = existingMemberId, Role = "Worker"
        });
        again.UserId.ShouldBe(existingMemberId);
    }
}
