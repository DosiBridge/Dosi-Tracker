using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Reqnroll;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;

namespace Dosi.Tracker.Acceptance.Features;

[Binding]
public sealed class ProjectsAndTeamSteps
{
    private readonly AcceptanceTestFixture _app;
    private readonly ScenarioState _state;

    public ProjectsAndTeamSteps(AcceptanceTestFixture app, ScenarioState state)
    {
        _app = app;
        _state = state;
    }

    private async Task<Guid> AddWorkerAsync()
    {
        var team = _app.Resolve<ITeamAppService>();
        var userId = Guid.NewGuid();
        var member = await team.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = _state.ProjectId,
            UserId = userId,
            Role = "Worker"
        });
        _state.MemberUserIds.Add(userId);
        _state.LastResult = member;
        return userId;
    }

    [Given(@"the workspace is on the ""(.*)"" plan")]
    public async Task GivenTheWorkspaceIsOnThePlan(string planName)
    {
        await _app.WithUow(async () =>
        {
            var plans = _app.Resolve<IRepository<Plan, Guid>>();
            var subscriptions = _app.Resolve<IRepository<Subscription, Guid>>();
            var guids = _app.Resolve<IGuidGenerator>();
            var plan = await plans.FirstAsync(p => p.Name == planName);
            await subscriptions.InsertAsync(new Subscription(guids.Create(), null, plan.Id));
        });
    }

    [Given(@"I have added a worker to the project")]
    public async Task GivenIHaveAddedAWorkerToTheProject()
    {
        await AddWorkerAsync();
    }

    [Given(@"I have added (\d+) workers to the project")]
    public async Task GivenIHaveAddedWorkersToTheProject(int count)
    {
        for (var i = 0; i < count; i++)
        {
            await AddWorkerAsync();
        }
    }

    [When(@"I list the members of the project")]
    public async Task WhenIListTheMembersOfTheProject()
    {
        var team = _app.Resolve<ITeamAppService>();
        _state.LastResult = await team.GetProjectMembersAsync(_state.ProjectId);
    }

    [When(@"I add a ""(.*)"" to the project")]
    public async Task WhenIAddAToTheProject(string role)
    {
        var team = _app.Resolve<ITeamAppService>();
        var userId = Guid.NewGuid();
        var member = await team.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = _state.ProjectId,
            UserId = userId,
            Role = role
        });
        _state.MemberUserIds.Add(userId);
        _state.LastResult = member;
    }

    [When(@"I add the same worker again")]
    public async Task WhenIAddTheSameWorkerAgain()
    {
        var team = _app.Resolve<ITeamAppService>();
        try
        {
            await team.AddMemberAsync(new CreateProjectMemberDto
            {
                ProjectId = _state.ProjectId,
                UserId = _state.MemberUserIds.Last(),
                Role = "Admin"
            });
        }
        catch (Exception ex)
        {
            _state.LastException = ex;
        }
    }

    [When(@"I add another worker")]
    public async Task WhenIAddAnotherWorker()
    {
        try
        {
            await AddWorkerAsync();
        }
        catch (Exception ex)
        {
            _state.LastException = ex;
        }
    }

    [When(@"I invite ""(.*)"" as a ""(.*)""")]
    public async Task WhenIInviteAsA(string email, string role)
    {
        var team = _app.Resolve<ITeamAppService>();
        _state.LastResult = await team.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = _state.ProjectId,
            Email = email,
            Role = role
        });
    }

    [Then(@"I am a member with the role ""(.*)""")]
    public void ThenIAmAMemberWithTheRole(string role)
    {
        var members = _state.Result<Volo.Abp.Application.Dtos.ListResultDto<ProjectMemberDto>>();
        members.Items.ShouldContain(m => m.UserId == ScenarioState.CurrentUserId && m.Role == role);
    }

    [Then(@"the project has (\d+) members")]
    public async Task ThenTheProjectHasMembers(int count)
    {
        var team = _app.Resolve<ITeamAppService>();
        var members = await team.GetProjectMembersAsync(_state.ProjectId);
        members.Items.Count.ShouldBe(count);
    }

    [Then(@"the new member has been notified")]
    public async Task ThenTheNewMemberHasBeenNotified()
    {
        var newMemberId = _state.MemberUserIds.Last();
        await _app.WithUow(async () =>
        {
            var notifications = _app.Resolve<IRepository<Notification, Guid>>();
            var forMember = await notifications.GetListAsync(n => n.UserId == newMemberId);
            forMember.ShouldNotBeEmpty();
        });
    }

    [Then(@"adding is rejected as a duplicate member")]
    public void ThenAddingIsRejectedAsADuplicateMember()
    {
        _state.LastException.ShouldBeOfType<BusinessException>()
            .Code.ShouldBe(TrackerDomainErrorCodes.DuplicateProjectMember);
    }

    [Then(@"adding is rejected because the seat limit is reached")]
    public void ThenAddingIsRejectedBecauseTheSeatLimitIsReached()
    {
        _state.LastException.ShouldBeOfType<BusinessException>()
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
    }

    [Then(@"a new user is created with a one-time password")]
    public void ThenANewUserIsCreatedWithAOneTimePassword()
    {
        var result = _state.Result<InviteMemberResultDto>();
        result.UserCreated.ShouldBeTrue();
        result.InitialPassword.ShouldNotBeNullOrWhiteSpace();
    }

    [Then(@"the invitee is a member of the project")]
    public async Task ThenTheInviteeIsAMemberOfTheProject()
    {
        var result = _state.Result<InviteMemberResultDto>();
        var team = _app.Resolve<ITeamAppService>();
        var members = await team.GetProjectMembersAsync(_state.ProjectId);
        members.Items.ShouldContain(m => m.UserId == result.Member.UserId);
    }
}
