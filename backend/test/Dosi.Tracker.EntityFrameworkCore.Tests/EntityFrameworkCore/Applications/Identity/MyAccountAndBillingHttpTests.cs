using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Identity;

/// <summary>
/// The account and billing self-service routes over HTTP: what they answer and to whom — the email confirmation and
/// deletion check need a signed-in caller, confirming an email works anonymously, a wrong password stops a deletion, the
/// billing mode is public, and only owners change the plan.
/// </summary>
public class MyAccountAndBillingHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private const string Password = "1q2w3E*http-account";
    private static readonly Guid Owner = Guid.Parse("d3701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Member = Guid.Parse("d3701e62-0953-4dd3-910b-dc6cc93ccb02");

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var connection = ServiceProvider.GetRequiredService<TrackerDbContext>().Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
            options.Configure(ctx => ctx.DbContextOptions.UseSqlite(connection)));
        services.AddAlwaysDisableUnitOfWorkTransaction();
    }

    private async Task<Guid> SeedAsync()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            foreach (var (id, userName) in new[] { (Owner, "http-account.owner"), (Member, "http-account.member") })
            {
                (await users.CreateAsync(new IdentityUser(id, userName, $"{userName}@acme.test"), Password)).Succeeded.ShouldBeTrue();
            }
        });

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("HTTP Account Co");
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        return workspaceId;
    }

    private static async Task<JsonElement> JsonAsync(HttpResponseMessage response)
    {
        var body = await response.Content.ReadAsStringAsync();
        response.IsSuccessStatusCode.ShouldBeTrue(body);
        using var document = JsonDocument.Parse(body);
        return document.RootElement.Clone();
    }

    [Fact]
    public async Task The_Account_Routes_Answer_The_Signed_In_Person_And_Confirming_Works_Anonymously()
    {
        await SeedAsync();
        var client = CreateHttpClient();

        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/my-account/deletion-check", null, null)))
            .StatusCode.ShouldBe(HttpStatusCode.Unauthorized);

        var ownerCheck = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/my-account/deletion-check", Owner, null)));
        ownerCheck.GetProperty("canDelete").GetBoolean().ShouldBeFalse();
        ownerCheck.GetProperty("soleOwnerOf")[0].GetProperty("name").GetString().ShouldBe("HTTP Account Co");

        var sent = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/my-account/send-email-confirmation", Member, null)));
        sent.GetProperty("emailSent").GetBoolean().ShouldBeTrue();

        var invalid = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/my-account/confirm-email", null, null,
            new { userId = Member, token = "forged" }));
        invalid.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        (await invalid.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.EmailConfirmationInvalid);

        var wrongPassword = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/my-account/delete", Member, null,
            new { password = "not-it" }));
        wrongPassword.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await wrongPassword.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvalidPassword);

        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/my-account/delete", Member, null, new { password = Password })))
            .StatusCode.ShouldBe(HttpStatusCode.NoContent);
        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByIdAsync(Member.ToString()))).ShouldBeNull();
    }

    [Fact]
    public async Task The_Billing_Mode_Is_Public_And_Only_Owners_Change_The_Plan()
    {
        var workspaceId = await SeedAsync();
        var client = CreateHttpClient();
        var free = await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Plan, Guid>>().FirstAsync(p => p.Name == Plan.FreePlanName));

        var mode = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/workspace/billing-mode", null, null)));
        mode.GetProperty("enabled").GetBoolean().ShouldBeTrue();

        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/billing/change-plan", Member, workspaceId, new { planId = free.Id })))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var changed = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/billing/change-plan", Owner, workspaceId,
            new { planId = free.Id })));
        changed.GetProperty("status").GetString().ShouldBe(Subscription.ActiveStatus);

        var cancel = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/billing/cancel", Owner, workspaceId));
        cancel.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await cancel.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.PlanChangeNotAllowed);
    }
}
