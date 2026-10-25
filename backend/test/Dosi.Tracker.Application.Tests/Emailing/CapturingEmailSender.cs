using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Net.Mail;
using System.Threading.Tasks;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Emailing;

namespace Dosi.Tracker.Emailing;

/// <summary>One email the application handed to the sender.</summary>
public sealed record CapturedEmail(string To, string Subject, string Body);

/// <summary>
/// The tests' email sender: nothing leaves the process — every email is only recorded. It also stands in for the
/// deployment's SMTP configuration (<see cref="CanDeliver"/>, on by default), so a test can switch email off.
/// </summary>
[DisableConventionalRegistration]
public sealed class CapturingEmailSender : IEmailSender, IEmailDeliveryChecker
{
    private readonly ConcurrentQueue<CapturedEmail> _emails = new();

    public bool CanDeliver { get; set; } = true;

    public IReadOnlyList<CapturedEmail> Emails => _emails.ToList();

    public IReadOnlyList<CapturedEmail> To(string address) =>
        _emails.Where(e => string.Equals(e.To, address, System.StringComparison.OrdinalIgnoreCase)).ToList();

    public Task<bool> CanDeliverAsync() => Task.FromResult(CanDeliver);

    public Task SendAsync(string to, string? subject, string? body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null)
    {
        _emails.Enqueue(new CapturedEmail(to, subject ?? string.Empty, body ?? string.Empty));
        return Task.CompletedTask;
    }

    public Task SendAsync(string from, string to, string? subject, string? body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null) =>
        SendAsync(to, subject, body, isBodyHtml, additionalEmailSendingArgs);

    public Task SendAsync(MailMessage mail, bool normalize = true) =>
        SendAsync(mail.To.ToString(), mail.Subject, mail.Body, mail.IsBodyHtml);

    public Task QueueAsync(string to, string subject, string body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null) =>
        SendAsync(to, subject, body, isBodyHtml, additionalEmailSendingArgs);

    public Task QueueAsync(string from, string to, string subject, string body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null) =>
        SendAsync(to, subject, body, isBodyHtml, additionalEmailSendingArgs);
}
