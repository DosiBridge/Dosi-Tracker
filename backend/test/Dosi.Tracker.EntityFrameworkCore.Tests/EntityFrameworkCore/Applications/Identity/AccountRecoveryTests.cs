using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Emailing;
using Dosi.Tracker.Identity;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Account;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Identity;

/// <summary>
/// Account recovery and email confirmation, end to end through the emailed links: a reset request answers the same for
/// any address and only emails an active account; the link opens the web app, keeps only a local return path, works,
/// and confirms the address; an owner can have a member emailed a reset link; sign-up and the member's own request send
/// a confirmation link that confirms the address once.
/// </summary>
public class AccountRecoveryTests : TrackerEntityFrameworkCoreTestBase
{
    private const string Password = "1q2w3E*recovery";

    private readonly IAccountAppService _accountAppService;
    private readonly IMyAccountAppService _myAccountAppService;
    private readonly IdentityUserManager _userManager;
    private readonly CapturingEmailSender _emails;
    private readonly ICurrentPrincipalAccessor _principalAccessor;
    private readonly ICurrentWorkspace _currentWorkspace;

    public AccountRecoveryTests()
    {
        _accountAppService = GetRequiredService<IAccountAppService>();
        _myAccountAppService = GetRequiredService<IMyAccountAppService>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _emails = GetRequiredService<CapturingEmailSender>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private IDisposable ActAs(Guid userId) =>
        _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(new[] { new Claim(AbpClaimTypes.UserId, userId.ToString()) }, "Test")));

    private Task<IdentityUser> CreateUserAsync(string email, bool active = true)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(GetRequiredService<IGuidGenerator>().Create(), email, email);
            (await _userManager.CreateAsync(user, Password)).Succeeded.ShouldBeTrue();
            if (!active)
            {
                user.SetIsActive(false);
                (await _userManager.UpdateAsync(user)).Succeeded.ShouldBeTrue();
            }

            return user;
        });
    }

    private Task<IdentityUser> ReloadAsync(Guid userId) => WithUnitOfWorkAsync(() => _userManager.GetByIdAsync(userId));

    [Fact]
    public async Task A_Reset_Request_Answers_The_Same_For_Any_Address_And_Emails_Only_An_Active_Account()
    {
        await CreateUserAsync("inactive@recovery.test", active: false);

        await _accountAppService.SendPasswordResetCodeAsync(new SendPasswordResetCodeDto { Email = "nobody@recovery.test", AppName = "WEB" });
        await _accountAppService.SendPasswordResetCodeAsync(new SendPasswordResetCodeDto { Email = "inactive@recovery.test", AppName = "WEB" });

        _emails.Emails.ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Emailed_Link_Resets_The_Password_And_Confirms_The_Address()
    {
        var user = await CreateUserAsync("forgot@recovery.test");
        user.EmailConfirmed.ShouldBeFalse();

        await _accountAppService.SendPasswordResetCodeAsync(new SendPasswordResetCodeDto
        {
            Email = "FORGOT@recovery.test",
            AppName = "WEB",
            ReturnUrl = "/team?tab=invitations"
        });

        var email = _emails.To("forgot@recovery.test").ShouldHaveSingleItem();
        email.Subject.ShouldBe("Reset your Dosi Tracker password");
        var link = email.LinkStartingWith(EmailLinkTestHelpers.WebApp + "/reset-password?");
        link.Query()["userId"].ShouldBe(user.Id.ToString("D"));
        link.Query()["returnUrl"].ShouldBe("/team?tab=invitations");

        await _accountAppService.ResetPasswordAsync(new ResetPasswordDto
        {
            UserId = user.Id,
            ResetToken = link.Query()["resetToken"]!,
            Password = "1q2w3E*brand-new"
        });

        var reloaded = await ReloadAsync(user.Id);
        (await WithUnitOfWorkAsync(() => _userManager.CheckPasswordAsync(reloaded, "1q2w3E*brand-new"))).ShouldBeTrue();
        reloaded.EmailConfirmed.ShouldBeTrue();
    }

    [Theory]
    [InlineData("//evil.example/steal")]
    [InlineData("https://evil.example")]
    public async Task A_Return_Url_Off_The_Web_App_Is_Dropped_From_The_Link(string returnUrl)
    {
        await CreateUserAsync("redirect@recovery.test");

        await _accountAppService.SendPasswordResetCodeAsync(new SendPasswordResetCodeDto
        {
            Email = "redirect@recovery.test",
            AppName = "WEB",
            ReturnUrl = returnUrl
        });

        var link = _emails.To("redirect@recovery.test").ShouldHaveSingleItem()
            .LinkStartingWith(EmailLinkTestHelpers.WebApp + "/reset-password?");
        link.Query()["returnUrl"].ShouldBeNull();
        link.Query()["resetToken"].ShouldNotBeNullOrWhiteSpace();
    }

    [Fact]
    public async Task An_Owner_Has_A_Member_Emailed_A_Reset_Link_But_Nobody_Else_Can()
    {
        var owner = await CreateUserAsync("owner@recovery.test");
        var manager = await CreateUserAsync("manager@recovery.test");
        var member = await CreateUserAsync("member@recovery.test");
        var outsider = await CreateUserAsync("outsider@recovery.test");
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Recovery Co");
        await ServiceProvider.AddWorkspaceMemberAsync(owner.Id, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(manager.Id, WorkspaceRoles.Manager, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(member.Id, WorkspaceRoles.Member, workspaceId);
        var team = GetRequiredService<ITeamAppService>();

        using (_currentWorkspace.Change(workspaceId))
        {
            using (ActAs(manager.Id))
            {
                (await Should.ThrowAsync<BusinessException>(() => team.SendPasswordResetAsync(member.Id)))
                    .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
            }

            using (ActAs(owner.Id))
            {
                (await Should.ThrowAsync<BusinessException>(() => team.SendPasswordResetAsync(outsider.Id)))
                    .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);

                (await team.SendPasswordResetAsync(member.Id)).EmailSent.ShouldBeTrue();

                _emails.CanDeliver = false;
                (await team.SendPasswordResetAsync(member.Id)).EmailSent.ShouldBeFalse();
            }
        }

        _emails.Emails.ShouldHaveSingleItem().To.ShouldBe("member@recovery.test");
        _emails.Emails[0].LinkStartingWith(EmailLinkTestHelpers.WebApp + "/reset-password?").Query()["userId"]
            .ShouldBe(member.Id.ToString("D"));
    }

    [Fact]
    public async Task The_Confirmation_Link_Confirms_The_Address_Once_And_A_Wrong_Link_Is_Refused()
    {
        var user = await CreateUserAsync("confirm@recovery.test");

        EmailConfirmationSentDto sent;
        using (ActAs(user.Id))
        {
            sent = await _myAccountAppService.SendEmailConfirmationAsync();
        }

        sent.EmailSent.ShouldBeTrue();
        sent.AlreadyConfirmed.ShouldBeFalse();
        var link = _emails.To("confirm@recovery.test").ShouldHaveSingleItem()
            .LinkStartingWith(EmailLinkTestHelpers.WebApp + "/confirm-email?");
        link.Query()["userId"].ShouldBe(user.Id.ToString("D"));

        (await Should.ThrowAsync<BusinessException>(() => _myAccountAppService.ConfirmEmailAsync(
            new ConfirmEmailDto { UserId = user.Id, Token = "not-the-token" }))).Code.ShouldBe(TrackerDomainErrorCodes.EmailConfirmationInvalid);
        (await Should.ThrowAsync<BusinessException>(() => _myAccountAppService.ConfirmEmailAsync(
            new ConfirmEmailDto { UserId = Guid.NewGuid(), Token = link.Query()["token"]! }))).Code.ShouldBe(TrackerDomainErrorCodes.EmailConfirmationInvalid);
        (await ReloadAsync(user.Id)).EmailConfirmed.ShouldBeFalse();

        var confirm = new ConfirmEmailDto { UserId = user.Id, Token = link.Query()["token"]! };
        await _myAccountAppService.ConfirmEmailAsync(confirm);
        await _myAccountAppService.ConfirmEmailAsync(confirm); // opening the link twice is fine

        (await ReloadAsync(user.Id)).EmailConfirmed.ShouldBeTrue();
        using (ActAs(user.Id))
        {
            (await _myAccountAppService.SendEmailConfirmationAsync()).AlreadyConfirmed.ShouldBeTrue();
        }

        _emails.Emails.Count.ShouldBe(1);
    }

    [Fact]
    public async Task Signing_Up_Sends_The_New_Account_A_Confirmation_Link()
    {
        WorkspaceRegistrationResultDto result;
        using (_currentWorkspace.Change(null))
        {
            result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
            {
                WorkspaceName = "Fresh Signup Co",
                Email = "fresh@recovery.test",
                Password = Password,
                FirstName = "Fresh"
            });
        }

        result.EmailConfirmationSent.ShouldBeTrue();
        _emails.To("fresh@recovery.test").ShouldHaveSingleItem().Subject.ShouldBe("Confirm your email address for Dosi Tracker");
    }

    [Fact]
    public async Task Without_Email_Delivery_Nothing_Is_Sent_And_That_Is_Reported()
    {
        var user = await CreateUserAsync("nomail@recovery.test");
        _emails.CanDeliver = false;

        using (ActAs(user.Id))
        {
            (await _myAccountAppService.SendEmailConfirmationAsync()).EmailSent.ShouldBeFalse();
        }

        await _accountAppService.SendPasswordResetCodeAsync(new SendPasswordResetCodeDto { Email = "nomail@recovery.test", AppName = "WEB" });
        _emails.Emails.ShouldBeEmpty();
    }
}
