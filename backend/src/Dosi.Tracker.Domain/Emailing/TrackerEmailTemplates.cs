using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Net;

namespace Dosi.Tracker.Emailing;

/// <summary>An email ready to send: its subject and HTML body.</summary>
public sealed record EmailContent(string Subject, string HtmlBody);

/// <summary>
/// The transactional emails: an invitation, a password reset, an email confirmation and the trial notices. Each is one
/// short message with one action link, shown as a button and as plain text for mail clients that drop buttons. Every
/// value that comes from people (names, workspace names) is HTML-encoded. Dates are UTC.
/// </summary>
public static class TrackerEmailTemplates
{
    public const string ProductName = "Dosi Tracker";

    public static EmailContent Invitation(
        string workspaceName, string? inviterName, string role, string acceptLink, DateTime expiresAt)
    {
        var inviter = string.IsNullOrWhiteSpace(inviterName) ? "A workspace owner" : inviterName.Trim();
        return Build(
            $"{inviter} invited you to {workspaceName} on {ProductName}",
            $"You're invited to {workspaceName}",
            new[]
            {
                $"{inviter} invited you to join the workspace {workspaceName} as {Article(role)} {role}.",
                $"The invitation expires on {FormatDate(expiresAt)} (UTC). Nobody joins a workspace without accepting."
            },
            "Accept the invitation",
            acceptLink,
            "You received this email because someone invited this address. If you did not expect it, you can ignore it.");
    }

    public static EmailContent PasswordReset(string resetLink)
    {
        return Build(
            $"Reset your {ProductName} password",
            "Reset your password",
            new[]
            {
                $"Someone asked to reset the password of the {ProductName} account that uses this email address.",
                "Open the link to choose a new password. It works once, and only for a limited time."
            },
            "Choose a new password",
            resetLink,
            "If you did not ask for this, ignore this email: your password stays as it is.");
    }

    public static EmailContent EmailConfirmation(string confirmLink)
    {
        return Build(
            $"Confirm your email address for {ProductName}",
            "Confirm your email address",
            new[]
            {
                $"Confirm that this address belongs to your {ProductName} account.",
                "Once it is confirmed, the invitations sent to it appear in the app, and you can accept them there."
            },
            "Confirm my email address",
            confirmLink,
            "If you did not create an account, ignore this email.");
    }

    public static EmailContent TrialEnding(string workspaceName, string planName, DateTime endsAt, string? billingLink)
    {
        return Build(
            $"Your {planName} trial of {workspaceName} ends on {FormatDate(endsAt)}",
            "Your trial ends soon",
            new[]
            {
                $"The {planName} trial of the workspace {workspaceName} ends on {FormatDate(endsAt)} (UTC).",
                "After it ends, time tracking pauses until the subscription is activated. If your team fits the free " +
                "plan, you can switch to it on the billing page instead."
            },
            billingLink == null ? null : "Open billing",
            billingLink,
            "You received this email because you own this workspace.");
    }

    public static EmailContent TrialEnded(string workspaceName, string planName, string? billingLink)
    {
        return Build(
            $"Your {planName} trial of {workspaceName} has ended",
            "Your trial has ended",
            new[]
            {
                $"The {planName} trial of the workspace {workspaceName} has ended, and time tracking is paused.",
                "Activate the subscription to continue, or switch to the free plan on the billing page if your team " +
                "fits it. Everything tracked so far is kept."
            },
            billingLink == null ? null : "Open billing",
            billingLink,
            "You received this email because you own this workspace.");
    }

    private static EmailContent Build(
        string subject, string title, IEnumerable<string> paragraphs, string? actionText, string? actionLink, string footer)
    {
        var body = string.Concat(paragraphs.Select(p => $"<p style=\"margin:0 0 16px;line-height:1.5\">{Encode(p)}</p>"));
        var action = actionText == null || actionLink == null
            ? string.Empty
            : $"<p style=\"margin:24px 0\"><a href=\"{Encode(actionLink)}\" style=\"background:#2563eb;color:#ffffff;" +
              "padding:12px 20px;border-radius:6px;text-decoration:none;display:inline-block\">" +
              $"{Encode(actionText)}</a></p>" +
              "<p style=\"margin:0 0 16px;font-size:13px;color:#52606d;line-height:1.5\">If the button does not work, " +
              $"copy this link into your browser:<br><a href=\"{Encode(actionLink)}\">{Encode(actionLink)}</a></p>";

        var html =
            "<!doctype html><html><body style=\"margin:0;padding:24px;background:#f5f6f8;" +
            "font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#1f2933\">" +
            "<div style=\"max-width:560px;margin:0 auto;background:#ffffff;border-radius:8px;padding:32px\">" +
            $"<p style=\"margin:0 0 16px;font-size:14px;color:#52606d\">{Encode(ProductName)}</p>" +
            $"<h1 style=\"margin:0 0 16px;font-size:20px\">{Encode(title)}</h1>" +
            body + action +
            $"<p style=\"margin:24px 0 0;font-size:12px;color:#7b8794;line-height:1.5\">{Encode(footer)}</p>" +
            "</div></body></html>";

        return new EmailContent(subject, html);
    }

    private static string Encode(string value) => WebUtility.HtmlEncode(value);

    private static string Article(string word) =>
        word.Length > 0 && "aeiouAEIOU".Contains(word[0]) ? "an" : "a";

    private static string FormatDate(DateTime value) =>
        value.ToString("MMMM d, yyyy", CultureInfo.InvariantCulture);
}
