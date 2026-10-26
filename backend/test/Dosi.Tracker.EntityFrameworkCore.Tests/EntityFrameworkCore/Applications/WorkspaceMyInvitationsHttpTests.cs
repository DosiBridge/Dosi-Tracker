using System;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>
/// The signed-in user's own invitations over the hosted HTTP pipeline (<c>/api/app/invitations/mine</c>): the literal
/// route wins over the token lookup, it needs a signed-in caller and no workspace, and an invitation is matched by the
/// account's email only once that email is verified — an unverified account (anyone can sign up with any address)
/// neither sees nor answers invitations without their token.
/// </summary>
public class WorkspaceMyInvitationsHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid VerifiedUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb41");
    private static readonly Guid UnverifiedUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb42");

    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;

    public WorkspaceMyInvitationsHttpTests()
    {
        _invitationRepository = GetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var db = ServiceProvider.GetRequiredService<TrackerDbContext>();
        var conn = db.Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
        {
            options.Configure(ctx =>
            {
                ctx.DbContextOptions.UseSqlite(conn);
            });
        });
        services.AddAlwaysDisableUnitOfWorkTransaction();
        services.AddControllers().AddApplicationPart(typeof(WorkspacesController).Assembly);
    }

    private async Task<Guid> SeedAsync(string workspaceName)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            foreach (var (id, name, confirmed) in new[] { (VerifiedUser, "verified", true), (UnverifiedUser, "unverified", false) })
            {
                if (await _userManager.FindByIdAsync(id.ToString()) == null)
                {
                    var user = new IdentityUser(id, name, $"{name}@acme.test");
                    user.SetEmailConfirmed(confirmed);
                    (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
                }
            }
        });

        return await ServiceProvider.CreateWorkspaceAsync(workspaceName);
    }

    private async Task<WorkspaceInvitation> InviteAsync(Guid workspaceId, string email, string role = WorkspaceRoles.Member)
    {
        var invitation = new WorkspaceInvitation(
            _guidGenerator.Create(), workspaceId, email, role, null, null, null,
            WorkspaceInvitation.HashToken(WorkspaceInvitation.NewToken()), DateTime.UtcNow.AddDays(7));
        await WithUnitOfWorkAsync(() => _invitationRepository.InsertAsync(invitation, autoSave: true));
        return invitation;
    }

    private Task<WorkspaceMember?> FindMemberAsync(Guid workspaceId, Guid userId) =>
        WithUnitOfWorkAsync(() => _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == workspaceId && m.UserId == userId));

    [Fact]
    public async Task A_Verified_Account_Should_List_And_Accept_Its_Invitations_By_Id()
    {
        var workspaceId = await SeedAsync("MINE-WS-A");
        var invitation = await InviteAsync(workspaceId, "VERIFIED@acme.test", WorkspaceRoles.Manager);
        await InviteAsync(workspaceId, "unverified@acme.test");
        var client = CreateHttpClient();

        var list = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/invitations/mine", userId: VerifiedUser, workspaceId: null));
        list.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var body = JsonDocument.Parse(await list.Content.ReadAsStringAsync()))
        {
            body.RootElement.GetProperty("emailConfirmed").GetBoolean().ShouldBeTrue();
            var items = body.RootElement.GetProperty("items");
            items.GetArrayLength().ShouldBe(1);
            items[0].GetProperty("id").GetGuid().ShouldBe(invitation.Id);
            items[0].GetProperty("workspaceName").GetString().ShouldBe("MINE-WS-A");
            items[0].GetProperty("role").GetString().ShouldBe(WorkspaceRoles.Manager);
        }

        var accept = await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/invitations/mine/{invitation.Id}/accept",
            userId: VerifiedUser, workspaceId: null));
        accept.StatusCode.ShouldBe(HttpStatusCode.OK);
        (await FindMemberAsync(workspaceId, VerifiedUser)).ShouldNotBeNull().Role.ShouldBe(WorkspaceRoles.Manager);
    }

    [Fact]
    public async Task An_Unverified_Account_Should_Neither_See_Nor_Answer_Invitations_Without_Their_Token()
    {
        var workspaceId = await SeedAsync("MINE-WS-B");
        var invitation = await InviteAsync(workspaceId, "unverified@acme.test", WorkspaceRoles.Owner);
        var client = CreateHttpClient();

        var list = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/invitations/mine", userId: UnverifiedUser, workspaceId: null));
        list.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var body = JsonDocument.Parse(await list.Content.ReadAsStringAsync()))
        {
            body.RootElement.GetProperty("emailConfirmed").GetBoolean().ShouldBeFalse();
            body.RootElement.GetProperty("items").GetArrayLength().ShouldBe(0);
        }

        foreach (var action in new[] { "accept", "decline" })
        {
            var response = await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/invitations/mine/{invitation.Id}/{action}",
                userId: UnverifiedUser, workspaceId: null));
            response.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
            (await response.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.EmailNotConfirmed);
        }

        (await FindMemberAsync(workspaceId, UnverifiedUser)).ShouldBeNull();
        (await WithUnitOfWorkAsync(() => _invitationRepository.FindAsync(invitation.Id))).ShouldNotBeNull();
    }

    [Fact]
    public async Task My_Invitations_Should_Require_A_Signed_In_Caller()
    {
        await SeedAsync("MINE-WS-C");
        var client = CreateHttpClient();

        var response = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/invitations/mine", userId: null, workspaceId: null));

        // Not the anonymous token lookup of an invitation called "mine".
        response.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
    }
}
