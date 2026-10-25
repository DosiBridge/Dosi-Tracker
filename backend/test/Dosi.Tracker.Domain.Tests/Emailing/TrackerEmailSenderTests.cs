using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using NSubstitute;
using NSubstitute.ExceptionExtensions;
using Shouldly;
using Volo.Abp.BackgroundJobs;
using Volo.Abp.Emailing;
using Volo.Abp.Emailing.Smtp;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// The application's sender queues email for background delivery (ABP's email job, which retries) only once SMTP is
/// configured; until then it sends nothing and logs that — with the recipient masked and never the body, which can carry
/// a sign-in link. A configuration that cannot be read counts as not configured.
/// </summary>
public class TrackerEmailSenderTests
{
    private const string Body = "<a href=\"https://app.test/reset-password?resetToken=SECRET-TOKEN\">reset</a>";

    private readonly IBackgroundJobManager _jobs = Substitute.For<IBackgroundJobManager>();
    private readonly IEmailDeliveryChecker _checker = Substitute.For<IEmailDeliveryChecker>();
    private readonly ListLogger<EmailSenderBase> _logger = new();

    private TrackerEmailSender NewSender()
    {
        return new TrackerEmailSender(
            Substitute.For<ICurrentTenant>(),
            Substitute.For<ISmtpEmailSenderConfiguration>(),
            _jobs,
            _checker)
        {
            Logger = _logger
        };
    }

    [Fact]
    public async Task Without_Smtp_Nothing_Is_Queued_And_The_Log_Names_Neither_The_Address_Nor_The_Link()
    {
        _checker.CanDeliverAsync().Returns(false);
        var sender = NewSender();

        await sender.QueueAsync("jane.doe@example.com", "Reset your password", Body);
        await sender.QueueAsync("noreply@example.com", "jane.doe@example.com", "Reset your password", Body);
        await sender.SendAsync("jane.doe@example.com", "Reset your password", Body);

        await _jobs.DidNotReceiveWithAnyArgs().EnqueueAsync<BackgroundEmailSendingJobArgs>(default!);
        _logger.Entries.Count.ShouldBe(3);
        foreach (var (level, message) in _logger.Entries)
        {
            level.ShouldBe(LogLevel.Warning);
            message.ShouldContain("j***@example.com");
            message.ShouldContain(TrackerSmtpEmailSenderConfiguration.HostConfigurationKey);
            message.ShouldNotContain("jane.doe");
            message.ShouldNotContain("SECRET-TOKEN");
        }
    }

    [Fact]
    public async Task With_Smtp_The_Email_Is_Queued_For_Background_Delivery()
    {
        _checker.CanDeliverAsync().Returns(true);
        var queued = new List<BackgroundEmailSendingJobArgs>();
        _jobs.EnqueueAsync(Arg.Do<BackgroundEmailSendingJobArgs>(queued.Add), Arg.Any<BackgroundJobPriority>(), Arg.Any<TimeSpan?>())
            .Returns("job-1");
        var sender = NewSender();

        await sender.QueueAsync("jane.doe@example.com", "Reset your password", Body);
        await sender.QueueAsync("noreply@example.com", "john@example.com", "Hello", "Hi");

        queued.Count.ShouldBe(2);
        queued[0].To.ShouldBe("jane.doe@example.com");
        queued[0].Body.ShouldBe(Body);
        queued[1].From.ShouldBe("noreply@example.com");
        _logger.Entries.ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Unreadable_Configuration_Counts_As_No_Smtp()
    {
        _checker.CanDeliverAsync().ThrowsAsync(new InvalidOperationException("the setting store is down"));
        var sender = NewSender();

        await sender.QueueAsync("jane.doe@example.com", "Subject", Body);

        await _jobs.DidNotReceiveWithAnyArgs().EnqueueAsync<BackgroundEmailSendingJobArgs>(default!);
        _logger.Entries.Count.ShouldBe(2); // the failure to read, and the email not sent
    }

    [Theory]
    [InlineData("jane.doe@example.com", "j***@example.com")]
    [InlineData("  x@y.org ", "x***@y.org")]
    [InlineData("no-at-sign", "***")]
    [InlineData("@example.com", "***")]
    [InlineData(null, "(none)")]
    [InlineData(" ", "(none)")]
    public void Addresses_Are_Masked_For_Logs(string? address, string expected)
    {
        EmailAddressMask.Mask(address).ShouldBe(expected);
    }
}
