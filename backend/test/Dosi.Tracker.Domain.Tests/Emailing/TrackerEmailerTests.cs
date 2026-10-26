using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using NSubstitute;
using NSubstitute.ExceptionExtensions;
using Shouldly;
using Volo.Abp.Emailing;
using Xunit;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// The transactional emails are best effort: queued only when email can be delivered and the link has a web app to point
/// to, reported as sent or not, and never an exception for the operation that sends them.
/// </summary>
public class TrackerEmailerTests
{
    private static readonly DateTime ExpiresAt = new(2026, 11, 9, 12, 0, 0, DateTimeKind.Utc);

    private readonly IEmailSender _sender = Substitute.For<IEmailSender>();
    private readonly IEmailDeliveryChecker _checker = Substitute.For<IEmailDeliveryChecker>();

    private TrackerEmailer NewEmailer(string? clientUrl = "https://app.example.com")
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?> { [WebAppLinks.ClientUrlKey] = clientUrl })
            .Build();
        return new TrackerEmailer(_sender, _checker, new WebAppLinks(configuration));
    }

    [Fact]
    public async Task Every_Email_Is_Queued_With_Its_Link_When_Email_Can_Be_Delivered()
    {
        _checker.CanDeliverAsync().Returns(true);
        var emailer = NewEmailer();
        var userId = Guid.Parse("4f6e1c1e-2d1b-4d55-9a3b-3b0d6d8f0a11");

        (await emailer.QueueInvitationAsync("ann@example.com", "Acme", "Bob Owner", "member", "tok123", ExpiresAt)).ShouldBeTrue();
        (await emailer.QueuePasswordResetAsync("ann@example.com", "https://app.example.com/reset-password?x=1")).ShouldBeTrue();
        (await emailer.QueueEmailConfirmationAsync("ann@example.com", userId, "a+b/c")).ShouldBeTrue();
        (await emailer.QueueTrialEndingAsync("ann@example.com", "Acme", "Starter", ExpiresAt)).ShouldBeTrue();
        (await emailer.QueueTrialEndedAsync("ann@example.com", "Acme", "Starter")).ShouldBeTrue();

        await _sender.Received(1).QueueAsync("ann@example.com", "Bob Owner invited you to Acme on Dosi Tracker",
            Arg.Is<string>(body => body.Contains("https://app.example.com/invite/tok123")), true);
        await _sender.Received(1).QueueAsync("ann@example.com", "Confirm your email address for Dosi Tracker",
            Arg.Is<string>(body => body.Contains($"https://app.example.com/confirm-email?userId={userId:D}&amp;token=a%2Bb%2Fc")), true);
        await _sender.Received(1).QueueAsync("ann@example.com", "Your Starter trial of Acme ends on November 9, 2026",
            Arg.Is<string>(body => body.Contains("https://app.example.com/billing")), true);
        await _sender.Received(1).QueueAsync("ann@example.com", "Your Starter trial of Acme has ended", Arg.Any<string>(), true);
    }

    [Fact]
    public async Task Nothing_Is_Queued_When_Email_Cannot_Be_Delivered_Or_Has_No_Address()
    {
        _checker.CanDeliverAsync().Returns(false);
        var emailer = NewEmailer();

        (await emailer.QueueInvitationAsync("ann@example.com", "Acme", null, "member", "tok", ExpiresAt)).ShouldBeFalse();
        (await emailer.CanDeliverAsync()).ShouldBeFalse();

        _checker.CanDeliverAsync().Returns(true);
        (await emailer.QueuePasswordResetAsync(" ", "https://app.example.com/reset")).ShouldBeFalse();

        await _sender.DidNotReceiveWithAnyArgs().QueueAsync(
            Arg.Any<string>(), Arg.Any<string>(), Arg.Any<string>(), Arg.Any<bool>(), Arg.Any<AdditionalEmailSendingArgs?>());
    }

    [Fact]
    public async Task A_Link_Email_Is_Not_Sent_Without_A_Web_App_Address()
    {
        _checker.CanDeliverAsync().Returns(true);
        var emailer = NewEmailer(clientUrl: null);

        (await emailer.QueueInvitationAsync("ann@example.com", "Acme", null, "member", "tok", ExpiresAt)).ShouldBeFalse();
        (await emailer.QueueEmailConfirmationAsync("ann@example.com", Guid.NewGuid(), "tok")).ShouldBeFalse();
        // A trial notice still goes out, without the billing button.
        (await emailer.QueueTrialEndedAsync("ann@example.com", "Acme", "Starter")).ShouldBeTrue();

        await _sender.Received(1).QueueAsync("ann@example.com", Arg.Any<string>(),
            Arg.Is<string>(body => !body.Contains("Open billing")), true);
    }

    [Fact]
    public async Task A_Failure_To_Queue_Or_To_Read_The_Configuration_Is_Reported_Not_Thrown()
    {
        _checker.CanDeliverAsync().Returns(true);
        _sender.QueueAsync(Arg.Any<string>(), Arg.Any<string>(), Arg.Any<string>(), Arg.Any<bool>(), Arg.Any<AdditionalEmailSendingArgs?>())
            .Returns(Task.FromException(new InvalidOperationException("store down")));
        (await NewEmailer().QueueTrialEndedAsync("ann@example.com", "Acme", "Starter")).ShouldBeFalse();

        _checker.CanDeliverAsync().ThrowsAsync(new InvalidOperationException("settings down"));
        (await NewEmailer().CanDeliverAsync()).ShouldBeFalse();
    }
}
