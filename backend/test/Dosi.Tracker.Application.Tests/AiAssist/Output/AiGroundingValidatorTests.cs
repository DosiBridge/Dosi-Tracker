using System;
using System.Collections.Generic;
using System.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>
/// <see cref="AiGroundingValidator"/>, <see cref="AiBannedTerms"/> and <see cref="AiRouteIds"/>: every output rule, with
/// guards against false positives ("Firefox", "may", "someone", "Slack").
/// </summary>
public class AiGroundingValidatorTests
{
    private static readonly IReadOnlySet<string> Known = new HashSet<string>(StringComparer.Ordinal)
    {
        "F1", "F2", "M1", "P1", "A1", "T1", "W1", "N1", "ME"
    };

    private static IReadOnlyList<AiViolation> Grounded(string text) =>
        AiGroundingValidator.ValidateText("text", text, AiValidationProfile.Grounded, Known);

    private static void ShouldPass(IReadOnlyList<AiViolation> violations) =>
        violations.ShouldBeEmpty(string.Join("; ", violations.Select(v => v.Message)));

    private static void ShouldFail(IReadOnlyList<AiViolation> violations, string messagePart) =>
        violations.ShouldContain(v => v.Path == "text" && v.Message.Contains(messagePart), string.Join("; ", violations.Select(v => v.Message)));

    [Theory]
    [InlineData("{M1} tracked {F1} on {P1}, mostly in Firefox.")]
    [InlineData("Someone may want to check {A1}; {ME} spent {F2} there.")]
    [InlineData("Time in Slack, the app, was {F2}.")]
    [InlineData("The terminal and {W1} were open; {N1} explains {T1}.")]
    [InlineData("- {ME} tracked {F1}\n- {M1} tracked {F2}")]
    public void Grounded_Text_Should_Pass(string text)
    {
        ShouldPass(Grounded(text));
    }

    [Fact]
    public void Empty_Text_Should_Have_Nothing_To_Check()
    {
        ShouldPass(Grounded(""));
        ShouldPass(AiGroundingValidator.ValidateText("text", null, AiValidationProfile.Chat, Known));
    }

    [Theory]
    [InlineData("{M1} tracked 6 hours.", "digit")]
    [InlineData("{M1} tracked ٣ hours.", "digit")]
    [InlineData("That is twice as much as {F1}.", "number word")]
    [InlineData("About half of {F1}.", "number word")]
    [InlineData("Ten percent of the time.", "number word")]
    [InlineData("On Monday {M1} tracked {F1}.", "weekday")]
    [InlineData("Since October {M1} tracked {F1}.", "month")]
    public void Grounded_Text_Should_Write_Values_Only_As_Placeholders(string text, string messagePart)
    {
        ShouldFail(Grounded(text), messagePart);
    }

    [Fact]
    public void May_Should_Not_Read_As_A_Month()
    {
        ShouldPass(Grounded("You may see {F1} in May as well."));
    }

    [Theory]
    [InlineData("{F9} was tracked.", "does not hold")]
    [InlineData("{P2} was busy.", "does not hold")]
    [InlineData("{F1234} was tracked.", "malformed")]
    [InlineData("{name} tracked {F1}.", "malformed")]
    [InlineData("{ F1 } was tracked.", "malformed")]
    [InlineData("{M1 tracked {F1}.", "stray brace")]
    [InlineData("{F1}} was tracked.", "stray brace")]
    public void Placeholders_Should_Be_Well_Formed_And_Known(string text, string messagePart)
    {
        ShouldFail(Grounded(text), messagePart);
    }

    [Fact]
    public void The_Reader_Should_Always_Be_A_Known_Placeholder()
    {
        var noReader = new HashSet<string> { "F1" };

        ShouldPass(AiGroundingValidator.ValidateText("text", "{ME} tracked {F1}.", AiValidationProfile.Grounded, noReader));
    }

    [Fact]
    public void Creative_Text_May_Repeat_Digits_From_Its_Inputs_Only()
    {
        var inputs = new[] { "Q3 planning", "Release 2.4" };

        ShouldPass(AiGroundingValidator.ValidateText("text", "Prepared the Q3 deck for {P1}", AiValidationProfile.Creative, Known, inputs));
        ShouldPass(AiGroundingValidator.ValidateText("text", "Shipped release 2.4", AiValidationProfile.Creative, Known, inputs));
        ShouldPass(AiGroundingValidator.ValidateText("text", "Draft two options in {A1}", AiValidationProfile.Creative, Known, inputs));
        ShouldFail(AiGroundingValidator.ValidateText("text", "Plan 2025 roadmap", AiValidationProfile.Creative, Known, inputs), "not in the inputs");
        ShouldFail(AiGroundingValidator.ValidateText("text", "Plan 7 tasks", AiValidationProfile.Creative, Known), "not in the inputs");
    }

    [Theory]
    [InlineData("Helped {M1} with {P1}")]
    [InlineData("{ME} reviewed {P1}")]
    public void Creative_Text_Should_Name_No_Person(string text)
    {
        ShouldFail(AiGroundingValidator.ValidateText("text", text, AiValidationProfile.Creative, Known), "names a person");
    }

    [Theory]
    [InlineData("{M1} looks lazy.")]
    [InlineData("{M1} was slacking.")]
    [InlineData("A clear underperformer.")]
    [InlineData("Poor   performance on {P1}.")]
    [InlineData("The top performer was {M1}.")]
    [InlineData("An unproductive member.")]
    [InlineData("Possible time theft.")]
    [InlineData("{M1} may be cheating.")]
    [InlineData("Looks fraudulent.")]
    [InlineData("Consider disciplinary steps.")]
    [InlineData("Consider termination.")]
    [InlineData("{M1} should be fired.")]
    [InlineData("Send a warning letter.")]
    [InlineData("{M1} ranked first.")]
    [InlineData("The ranking of the team.")]
    public void Judging_Language_Should_Be_Refused(string text)
    {
        ShouldFail(Grounded(text), "judging");
        AiBannedTerms.Contains(text).ShouldBeTrue();
    }

    [Theory]
    [InlineData("Slack was open.")]
    [InlineData("The terminal window.")]
    [InlineData("A frank review.")]
    [InlineData(null)]
    public void Ordinary_Words_Should_Not_Read_As_Judging(string? text)
    {
        AiBannedTerms.Contains(text).ShouldBeFalse();
    }

    [Theory]
    [InlineData("See https://example.com for {F1}.", "link")]
    [InlineData("See www.example.com for {F1}.", "link")]
    [InlineData("<b>{F1}</b>", "markup")]
    [InlineData("# Summary", "markup")]
    [InlineData("**{F1}** tracked", "markup")]
    [InlineData("`{F1}` tracked", "markup")]
    public void Links_And_Markup_Should_Be_Refused(string text, string messagePart)
    {
        ShouldFail(Grounded(text), messagePart);
    }

    [Fact]
    public void Chat_Answers_Should_Be_Short_And_Have_No_Headings()
    {
        ShouldPass(AiGroundingValidator.ValidateText("text", "These are the figures:\n- {ME} tracked {F1}", AiValidationProfile.Chat, Known));

        var tooLong = string.Concat(Enumerable.Repeat("{F1} ", AiAssistConsts.MaxAnswerLength / 5 + 1));
        ShouldFail(AiGroundingValidator.ValidateText("text", tooLong, AiValidationProfile.Chat, Known), "longer than");
        ShouldFail(AiGroundingValidator.ValidateText("text", "Summary\n=====\n{F1}", AiValidationProfile.Chat, Known), "heading");
        ShouldFail(AiGroundingValidator.ValidateText("text", "Summary\n---\n{F1}", AiValidationProfile.Chat, Known), "heading");
        ShouldFail(AiGroundingValidator.ValidateText("text", "{ME} tracked three hours.", AiValidationProfile.Chat, Known), "number word");
    }

    [Fact]
    public void Messages_Should_Never_Quote_The_Output()
    {
        var violations = Grounded("Visit https://evil.example/secret-path on Monday, 42 times, lazy {X1} {F77}");

        violations.ShouldNotBeEmpty();
        violations.ShouldAllBe(v => !v.Message.Contains("evil") && !v.Message.Contains("secret-path") && !v.Message.Contains("42"));
    }

    [Fact]
    public void Fact_References_Should_Name_Facts_Of_The_Data()
    {
        AiGroundingValidator.ValidateFactRefs("insights[0].factRefs", new[] { "F1", "{F2}", " F1 " }, Known).ShouldBeEmpty();

        var violations = AiGroundingValidator.ValidateFactRefs("insights[0].factRefs", new[] { "F1", "P1", "F9", null, "" }, Known);
        violations.Select(v => v.Path).ShouldBe(new[]
        {
            "insights[0].factRefs[1]", "insights[0].factRefs[2]", "insights[0].factRefs[3]", "insights[0].factRefs[4]"
        });
        AiGroundingValidator.ValidateFactRefs("refs", null, Known).ShouldBeEmpty();
    }

    [Fact]
    public void Route_Ids_Should_Be_Allowlisted_And_Offered()
    {
        var offered = new[] { AiRouteIds.Download, "admin" };

        AiGroundingValidator.ValidateRouteId("routeId", "", offered).ShouldBeNull();
        AiGroundingValidator.ValidateRouteId("routeId", null, offered, allowEmpty: false).ShouldNotBeNull();
        AiGroundingValidator.ValidateRouteId("routeId", AiRouteIds.Download, offered).ShouldBeNull();
        AiGroundingValidator.ValidateRouteId("routeId", AiRouteIds.Team, offered)!.Path.ShouldBe("routeId"); // allowlisted, not offered
        AiGroundingValidator.ValidateRouteId("routeId", "admin", offered).ShouldNotBeNull();               // offered, not allowlisted
    }

    [Fact]
    public void Choices_And_Required_Values_Should_Be_Checked()
    {
        var allowed = new[] { "c1", "c2" };

        AiGroundingValidator.ValidateChoice("candidateId", "c2", allowed).ShouldBeNull();
        AiGroundingValidator.ValidateChoice("candidateId", "c9", allowed).ShouldNotBeNull();
        AiGroundingValidator.ValidateChoice("candidateId", "", allowed).ShouldNotBeNull();
        AiGroundingValidator.ValidateChoice("actionId", "", allowed, allowEmpty: true).ShouldBeNull();

        AiGroundingValidator.ValidateRequired("headline", " ")!.Message.ShouldBe("is required");
        AiGroundingValidator.ValidateRequired("headline", "{F1}").ShouldBeNull();
    }

    [Fact]
    public void PlaceholderIds_Should_List_Each_Id_Once_In_Order()
    {
        AiGroundingValidator.PlaceholderIds("{M1} and {F2}, then {M1} and {ME} {bad}").ShouldBe(new[] { "M1", "F2", "ME" });
        AiGroundingValidator.PlaceholderIds(null).ShouldBeEmpty();
    }

    [Fact]
    public void Cap_Should_Cut_With_An_Ellipsis_And_Never_Inside_A_Placeholder()
    {
        AiGroundingValidator.Cap("  short  ", 10).ShouldBe("short");
        AiGroundingValidator.Cap("Tracked time grew", 10).ShouldBe("Tracked t…");
        AiGroundingValidator.Cap("Tracked  time grew", 9).ShouldBe("Tracked…"); // no space before the ellipsis
        AiGroundingValidator.Cap("Total was {F12} today", 13).ShouldBe("Total was…");
        AiGroundingValidator.Cap("Total {F1}", 10).ShouldBe("Total {F1}");
        AiGroundingValidator.Cap("abc", 1).ShouldBe("…");
        AiGroundingValidator.Cap("abc", 0).ShouldBe("");
        AiGroundingValidator.Cap(null, 5).ShouldBe("");
    }

    [Fact]
    public void CapList_Should_Drop_Extra_Items()
    {
        AiGroundingValidator.CapList(new[] { 1, 2, 3, 4 }, 3).ShouldBe(new[] { 1, 2, 3 });
        AiGroundingValidator.CapList<int>(null, 3).ShouldBeEmpty();
        AiGroundingValidator.CapList(new[] { 1 }, -1).ShouldBeEmpty();
    }

    [Fact]
    public void Route_Ids_Should_Cover_The_Pages_And_Every_Report()
    {
        foreach (var id in new[]
                 {
                     "dashboard", "monitor", "timesheet", "team", "projects", "projects.detail", "activities", "download",
                     "install.windows", "install.macos", "install.linux", "settings", "settings.workspace", "settings.privacy",
                     "reports", "billing"
                 })
        {
            AiRouteIds.IsAllowed(id).ShouldBeTrue(id);
        }

        AiRouteIds.ReportSlugs.ShouldAllBe(slug => AiRouteIds.IsAllowed(AiRouteIds.ForReport(slug)));
        AiRouteIds.ForReport("weekly").ShouldBe("reports.weekly");
        AiRouteIds.All.Count.ShouldBe(16 + AiRouteIds.ReportSlugs.Count);

        AiRouteIds.IsAllowed("Dashboard").ShouldBeFalse();
        AiRouteIds.IsAllowed("reports.unknown").ShouldBeFalse();
        AiRouteIds.IsAllowed("/dashboard").ShouldBeFalse();
        AiRouteIds.IsAllowed(null).ShouldBeFalse();
    }
}
