using System;
using System.Collections.Specialized;
using System.Net;
using System.Text.RegularExpressions;
using System.Web;
using Dosi.Tracker.Emailing;
using Shouldly;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Identity;

/// <summary>Reads the action link out of a captured email, as the person would click it.</summary>
public static class EmailLinkTestHelpers
{
    public const string WebApp = "https://app.dosi.test";

    /// <summary>The first link of the email that starts with <paramref name="prefix"/>, HTML-decoded.</summary>
    public static Uri LinkStartingWith(this CapturedEmail email, string prefix)
    {
        var match = Regex.Match(email.Body, "href=\"(?<link>" + Regex.Escape(WebUtility.HtmlEncode(prefix)) + "[^\"]*)\"");
        match.Success.ShouldBeTrue($"No link starting with {prefix} in: {email.Body}");
        return new Uri(WebUtility.HtmlDecode(match.Groups["link"].Value));
    }

    public static NameValueCollection Query(this Uri link) => HttpUtility.ParseQueryString(link.Query);
}
