using System;
using System.Collections.Generic;
using Microsoft.Extensions.Configuration;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Emailing;

/// <summary>Email links point into the web app (App:ClientUrl, else the first CORS origin), escape their tokens and
/// only carry a return URL that stays on the web app; templates HTML-encode everything people typed.</summary>
public class WebAppLinksAndTemplatesTests
{
    private static IConfiguration Configuration(string? clientUrl, string? corsOrigins = null) =>
        new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
        {
            [WebAppLinks.ClientUrlKey] = clientUrl,
            [WebAppLinks.CorsOriginsKey] = corsOrigins
        }).Build();

    [Theory]
    [InlineData("https://app.example.com/", null, "https://app.example.com")]
    [InlineData("https://example.com/tracker/", null, "https://example.com/tracker")]
    [InlineData(null, "https://*.example.com, http://localhost:6005", "http://localhost:6005")]
    [InlineData(" ", "https://web.example.com", "https://web.example.com")]
    [InlineData("not a url", null, null)]
    [InlineData("ftp://files.example.com", null, null)]
    [InlineData(null, null, null)]
    public void The_Web_App_Address_Comes_From_The_Client_Url_Or_The_First_Cors_Origin(
        string? clientUrl, string? corsOrigins, string? expected)
    {
        WebAppLinks.ResolveBaseUrl(Configuration(clientUrl, corsOrigins)).ShouldBe(expected);
    }

    [Fact]
    public void Links_Escape_Their_Tokens()
    {
        var links = new WebAppLinks(Configuration("https://app.example.com"));
        var userId = Guid.Parse("4f6e1c1e-2d1b-4d55-9a3b-3b0d6d8f0a11");

        links.Invitation("ab/cd").ShouldBe("https://app.example.com/invite/ab%2Fcd");
        links.ResetPassword(userId, "t+k=")
            .ShouldBe($"https://app.example.com/reset-password?userId={userId:D}&resetToken=t%2Bk%3D");
        links.ResetPassword(userId, "tok", "/team?tab=1")
            .ShouldBe($"https://app.example.com/reset-password?userId={userId:D}&resetToken=tok&returnUrl=%2Fteam%3Ftab%3D1");
        links.ConfirmEmail(userId, "x y").ShouldBe($"https://app.example.com/confirm-email?userId={userId:D}&token=x%20y");
        links.Billing().ShouldBe("https://app.example.com/billing");

        var none = new WebAppLinks(Configuration(null));
        none.BaseUrl.ShouldBeNull();
        none.Invitation("tok").ShouldBeNull();
        none.Billing().ShouldBeNull();
    }

    [Theory]
    [InlineData("/team", "/team")]
    [InlineData("  /reports?range=week ", "/reports?range=week")]
    [InlineData("//evil.example", null)]
    [InlineData("/\\evil.example", null)]
    [InlineData("https://evil.example", null)]
    [InlineData("javascript:alert(1)", null)]
    [InlineData("/team\r\nSet-Cookie: x", null)]
    [InlineData("", null)]
    [InlineData(null, null)]
    public void Only_A_Path_On_The_Web_App_Is_A_Safe_Return_Url(string? returnUrl, string? expected)
    {
        WebAppLinks.SafeReturnUrl(returnUrl).ShouldBe(expected);
    }

    [Fact]
    public void Templates_Encode_What_People_Typed_And_Show_The_Link_Twice()
    {
        var email = TrackerEmailTemplates.Invitation(
            "<script>alert(1)</script>", "Eve \"Owner\"", "owner", "https://app.example.com/invite/t?a=1&b=2",
            new DateTime(2026, 11, 9, 0, 0, 0, DateTimeKind.Utc));

        email.Subject.ShouldBe("Eve \"Owner\" invited you to <script>alert(1)</script> on Dosi Tracker");
        email.HtmlBody.ShouldNotContain("<script>");
        email.HtmlBody.ShouldContain("&lt;script&gt;alert(1)&lt;/script&gt;");
        email.HtmlBody.ShouldContain("as an owner.");
        email.HtmlBody.ShouldContain("November 9, 2026");
        email.HtmlBody.Split("https://app.example.com/invite/t?a=1&amp;b=2").Length.ShouldBe(4); // button + text link
    }

    [Fact]
    public void Every_Template_Names_The_Product_And_Its_Action()
    {
        TrackerEmailTemplates.Invitation("Acme", null, "member", "https://x/invite", DateTime.UtcNow)
            .Subject.ShouldBe("A workspace owner invited you to Acme on Dosi Tracker");
        TrackerEmailTemplates.PasswordReset("https://x/reset").HtmlBody.ShouldContain("Choose a new password");
        TrackerEmailTemplates.EmailConfirmation("https://x/confirm").HtmlBody.ShouldContain("Confirm my email address");
        TrackerEmailTemplates.TrialEnding("Acme", "Starter", DateTime.UtcNow, null).HtmlBody.ShouldNotContain("Open billing");
        TrackerEmailTemplates.TrialEnded("Acme", "Starter", "https://x/billing").HtmlBody.ShouldContain("Open billing");
    }
}
