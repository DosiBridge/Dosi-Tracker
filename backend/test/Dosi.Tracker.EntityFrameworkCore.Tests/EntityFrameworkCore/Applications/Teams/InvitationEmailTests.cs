using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Emailing;
using Dosi.Tracker.EntityFrameworkCore.Applications.Identity;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Teams;

/// <summary>
/// Invitations are emailed (best effort): the email's accept link is the invitation's own, the answer says whether it was
/// sent, and resending renews the link — the old one stops working — and the expiry. Only owners hand out a new link for
/// more than the member role.
/// </summary>
public class InvitationEmailTests : TeamTestBase<TrackerEntityFrameworkCoreTestModule>
{
    private CapturingEmailSender Emails => GetRequiredService<CapturingEmailSender>();

    [Fact]
    public async Task An_Invitation_Is_Emailed_With_Its_Own_Accept_Link()
    {
        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = "newcomer@invite-mail.test" });

        result.EmailSent.ShouldBeTrue();
        var email = Emails.To("newcomer@invite-mail.test").ShouldHaveSingleItem();
        email.Subject.ShouldBe($"CALLER invited you to {TrackerTestConsts.WorkspaceName} on Dosi Tracker");
        email.LinkStartingWith(EmailLinkTestHelpers.WebApp + "/invite/").AbsolutePath.ShouldBe(result.InvitePath);
    }

    [Fact]
    public async Task Without_Email_Delivery_The_Invitation_Still_Works_And_Says_It_Was_Not_Sent()
    {
        Emails.CanDeliver = false;

        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = "copy-the-link@invite-mail.test" });

        result.EmailSent.ShouldBeFalse();
        result.InvitePath.ShouldStartWith("/invite/");
        Emails.Emails.ShouldBeEmpty();
    }

    [Fact]
    public async Task Resending_Renews_The_Link_And_The_Expiry_And_Emails_It_Again()
    {
        var first = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = "slow-reader@invite-mail.test" });
        var invitationRepository = GetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        await WithUnitOfWorkAsync(async () =>
        {
            var invitation = await invitationRepository.GetAsync(first.InvitationId!.Value);
            invitation.Renew(invitation.TokenHash, DateTime.UtcNow.AddMinutes(-1)); // it has expired meanwhile
            await invitationRepository.UpdateAsync(invitation, autoSave: true);
        });

        var resent = await TeamAppService.ResendInvitationAsync(first.InvitationId!.Value);

        resent.InvitationId.ShouldBe(first.InvitationId);
        resent.InvitePath.ShouldNotBe(first.InvitePath);
        resent.ExpiresAt!.Value.ShouldBeGreaterThan(DateTime.UtcNow.AddDays(WorkspaceConsts.InvitationLifetimeDays - 1));
        resent.EmailSent.ShouldBeTrue();
        Emails.To("slow-reader@invite-mail.test").Last().LinkStartingWith(EmailLinkTestHelpers.WebApp + "/invite/")
            .AbsolutePath.ShouldBe(resent.InvitePath);

        var workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        var oldToken = first.InvitePath!["/invite/".Length..];
        var newToken = resent.InvitePath!["/invite/".Length..];
        (await Should.ThrowAsync<BusinessException>(() => workspaceAppService.GetInvitationAsync(oldToken)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        (await workspaceAppService.GetInvitationAsync(newToken)).Expired.ShouldBeFalse();
    }

    [Fact]
    public async Task Only_Owners_Resend_An_Invitation_That_Grants_More_Than_The_Member_Role()
    {
        var managerInvite = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            Email = "future-manager@invite-mail.test",
            WorkspaceRole = WorkspaceRoles.Manager
        });
        var memberInvite = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = "future-member@invite-mail.test" });

        var manager = await CreateUserAsync("resend-manager", WorkspaceRoles.Manager);
        await AsCallerAsync(manager.Id, async () =>
        {
            (await Should.ThrowAsync<BusinessException>(() => TeamAppService.ResendInvitationAsync(managerInvite.InvitationId!.Value)))
                .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
            (await TeamAppService.ResendInvitationAsync(memberInvite.InvitationId!.Value)).EmailSent.ShouldBeTrue();
        });

        await Should.ThrowAsync<EntityNotFoundException>(() => TeamAppService.ResendInvitationAsync(Guid.NewGuid()));
    }

    private async Task AsCallerAsync(Guid userId, Func<Task> action)
    {
        var claims = new[] { new System.Security.Claims.Claim(Volo.Abp.Security.Claims.AbpClaimTypes.UserId, userId.ToString()) };
        using (GetRequiredService<Volo.Abp.Security.Claims.ICurrentPrincipalAccessor>()
                   .Change(new System.Security.Claims.ClaimsPrincipal(new System.Security.Claims.ClaimsIdentity(claims, "TestAuth"))))
        {
            await action();
        }
    }
}
