using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Read;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>
/// The pure parts of the AI cards: the fact helpers' caveats, folds and spans, the shared output checks, and the
/// deterministic answers of every data-quality kind, built from hand-made read models and bundles.
/// </summary>
public class AiFeaturePartsTests
{
    private static readonly Guid Caller = Guid.Parse("00000000-0000-4000-8000-0000000000c0");
    private static readonly Guid Ana = Guid.Parse("00000000-0000-4000-8000-0000000000a1");
    private static readonly DateTime Monday = new(2026, 3, 2, 0, 0, 0, DateTimeKind.Utc);

    private static AiSummary Summary(double tracked, params AiProjectRow[] projects) =>
        new(3, tracked, 10, 65, Array.Empty<AiMemberRow>(), projects, Array.Empty<AiMemberProjectRow>());

    private static AiBlock Block(DateTime start, int minutes, int? productivity = 60, int? injected = null, DateTime? end = null) =>
        new(Guid.NewGuid(), Ana, Guid.NewGuid(), start, end ?? start.AddMinutes(minutes), productivity, 0, "interval", 0, injected,
            Array.Empty<AiAppSeconds>(), null, null);

    private static string Display(AiContextBundle bundle, JsonObject totals, string key) => bundle.Facts[(string)totals[key]!].Display;

    [Fact]
    public void Period_Facts_Should_Add_Their_Caveats_Changes_And_Folds()
    {
        var writer = new AiBundleWriter(Caller);
        var totals = new JsonObject();
        var website = new AiProjectRow(Guid.NewGuid(), "Website", IsArchived: true, 2, 90, 0, 70);
        var others = Enumerable.Range(0, 3).Select(i => new AiProjectRow(Guid.NewGuid(), "Other " + i, false, 1, 10, 0, null)).ToArray();

        AiPeriodFacts.AddTotals(writer, totals, Summary(120, others.Prepend(website).ToArray()));
        AiPeriodFacts.AddRange(writer, totals, Monday, Monday.AddDays(7), TimeZoneInfo.Utc, Monday.AddDays(2));
        AiPeriodFacts.AddComparison(writer, totals, Summary(120), Summary(100));
        AiPeriodFacts.AddUnmeasured(writer, totals, new[] { new AiDataQualityRow(Ana, "ana", 3, 120, 30, null, 0, null) });
        AiPeriodFacts.AddDays(writer, new[]
        {
            new AiDailyRow(Monday, 1, 30, 0, 50), new AiDailyRow(Monday.AddDays(1), 2, 90, 0, 50), new AiDailyRow(Monday.AddDays(2), 0, 0, 0, null)
        }, withYear: false);
        AiPeriodFacts.AddProjects(writer, Summary(120, others.Prepend(website).ToArray()).Projects, 120, max: 2);
        AiPeriodFacts.AddApps(writer, Array.Empty<AiAppUsageRow>(), 5);
        var bundle = writer.Build();

        Display(bundle, totals, AiPeriodFacts.Period).ShouldBe("Mon 2 Mar to Sun 8 Mar");
        Display(bundle, totals, AiPeriodFacts.ChangePercent).ShouldBe("+20%");
        Display(bundle, totals, AiPeriodFacts.UnmeasuredShare).ShouldBe("25%");
        bundle.Caveats.ShouldBe(new[] { AiPeriodFacts.IncludesTodayCaveat, AiPeriodFacts.UnmeasuredCaveat, AiPeriodFacts.ArchivedProjectsCaveat });
        bundle.Facts[AiFeatureOutput.ListValue(bundle, AiPeriodFacts.BusiestDayList, "date")!].Display.ShouldBe("Tue 3 Mar");
        bundle.Facts[AiFeatureOutput.ListValue(bundle, AiPeriodFacts.LightestDayList, "date")!].Display.ShouldBe("Mon 2 Mar");
        bundle.Facts[AiFeatureOutput.ListValue(bundle, AiPeriodFacts.OtherProjectsList, "count")!].Display.ShouldBe("2");
        bundle.Lists.ContainsKey(AiPeriodFacts.AppsList).ShouldBeFalse();
        AiPeriodFacts.RangeDays(Monday, Monday.AddDays(7).AddHours(1)).ShouldBe(8);
    }

    [Fact]
    public void Day_Facts_Should_Merge_Short_Gaps_And_Report_Measurement_And_The_Previous_Week()
    {
        var writer = new AiBundleWriter(Caller);
        var totals = new JsonObject();
        var nine = Monday.AddHours(9);
        var blocks = new[]
        {
            Block(nine, 30),
            Block(nine.AddMinutes(31), 29, productivity: null),             // a one-minute gap: one stretch
            Block(nine.AddMinutes(40), 10, injected: 4),                     // inside the stretch
            Block(nine.AddHours(2), 0, end: nine.AddHours(2).AddMinutes(-5)) // ends before it starts: zero length
        };

        AiDayFacts.AddDay(writer, totals, Monday, TimeZoneInfo.Utc, Monday, Monday.AddHours(15));
        AiDayFacts.AddRhythm(writer, totals, blocks, TimeZoneInfo.Utc, "ME");
        AiDayFacts.AddRhythm(writer, new JsonObject(), Array.Empty<AiBlock>(), TimeZoneInfo.Utc, "ME");
        AiDayFacts.AddMeasurement(writer, totals, blocks);
        AiDayFacts.AddDayParts(writer, Array.Empty<AiHourlyRow>());
        AiDayFacts.AddWeekComparison(writer, totals, new[]
        {
            new AiMemberDayRow(Ana, "ana", Monday.AddDays(-3), 1, 120, 0, Monday, Monday),
            new AiMemberDayRow(Ana, "ana", Monday.AddDays(-2), 1, 60, 0, Monday, Monday),
            new AiMemberDayRow(Caller, "me", Monday.AddDays(-2), 1, 600, 0, Monday, Monday)
        }, Ana, 60, "M1");
        var bundle = writer.Build();

        Display(bundle, totals, AiDayFacts.LongestSpan).ShouldBe("1 h");
        Display(bundle, totals, AiDayFacts.Breaks).ShouldBe("1");
        Display(bundle, totals, AiPeriodFacts.UnmeasuredShare).ShouldBe("42%");
        Display(bundle, totals, AiDayFacts.SoftwareInputBlocks).ShouldBe("1");
        Display(bundle, totals, AiDayFacts.WeekActiveDays).ShouldBe("2");
        Display(bundle, totals, AiDayFacts.WeekAverage).ShouldBe("1 h 30 min");
        Display(bundle, totals, AiDayFacts.WeekDelta).ShouldBe("-30 min");
        bundle.Caveats.ShouldContain(c => c.StartsWith("The day is today"));
        bundle.Caveats.ShouldContain(AiDayFacts.SoftwareInputCaveat);
        bundle.Lists.ContainsKey(AiDayFacts.DayPartsList).ShouldBeFalse();
        AiDayFacts.Spans(blocks).Count.ShouldBe(2);
    }

    [Fact]
    public void Report_Facts_Should_Respect_Their_Row_Caps_And_Skip_Members_Without_A_Handle()
    {
        var members = Enumerable.Range(1, 30).Select(i => Guid.Parse($"00000000-0000-4000-8000-0000000001{i:00}")).ToList();
        var writer = new AiBundleWriter(AiPseudonymizer.ForCard(Caller, members.Select((m, i) => new AiMemberWeight(m, 100 - i)), max: 30));
        var project = new AiProjectRow(Guid.NewGuid(), "Website", false, 1, 60, 0, 60);
        var names = new Dictionary<Guid, string>();

        var portal = Guid.NewGuid();
        AiReportFacts.AddMemberProjects(writer,
            members.SelectMany(m => new[]
            {
                new AiMemberProjectRow(m, "x", project.ProjectId, "Website", 1, 5, 0, null), new AiMemberProjectRow(m, "x", portal, "Portal", 1, 5, 0, null)
            }).ToList(),
            new[] { project.ProjectId, portal });
        AiReportFacts.AddDevices(writer, members.Select(m => new AiDeviceRow(m, "user", Guid.NewGuid(), "Windows ", "v2", Monday, 5)).ToList(), names);
        AiReportFacts.AddAttendance(writer, new AiAttendance(Array.Empty<DateTime>(),
            new[] { new AiAttendanceRow(Guid.NewGuid(), "stranger", 1, 0, 0, 30, 0, null, null) }), names);
        var bundle = writer.Build();

        AiFeatureOutput.ListItems(bundle, AiReportFacts.MemberProjectsList).Count.ShouldBe(AiReportFacts.MaxMemberProjectRows);
        var devices = AiFeatureOutput.ListItems(bundle, AiReportFacts.DevicesList);
        devices.Count.ShouldBe(AiReportFacts.MaxDeviceRows);
        bundle.Facts[AiFeatureOutput.Value(devices[0], "platform")!].Display.ShouldBe("windows");
        bundle.Facts[AiFeatureOutput.Value(devices[0], "agentVersion")!].Display.ShouldBe("unknown");
        bundle.Lists.ContainsKey(AiReportFacts.AttendanceList).ShouldBeFalse("a member without a handle is left out");
        AiReportFacts.Platform("BeOS").ShouldBe("unknown");
        AiReportFacts.ParseVersion("3").ShouldBe(new Version(3, 0));
        AiReportFacts.ParseVersion(null).ShouldBeNull();
        AiReportFacts.MainListFor("weekly").ShouldBe(AiPeriodFacts.MembersList);
    }

    [Fact]
    public void The_Shared_Checks_Should_Flag_Missing_Items_Kinds_And_Routes()
    {
        var writer = new AiBundleWriter(Caller);
        var fact = writer.AddFact("tracked time", "1 h");
        writer.AddRoute(AiRouteIds.Team);
        var bundle = writer.Build();
        var violations = new List<AiViolation>();

        AiFeatureOutput.ValidateText(violations, "headline", " ", bundle, required: true);
        AiFeatureOutput.ValidateInsights(violations, "insights", new AiRawInsight?[]
        {
            null,
            new("ranking", "{F1} in total.", new[] { fact }, "billing"),
            new("trend", "{F1} in total.", new[] { fact }, "team")
        }, 5, AiInsightKinds.Period, bundle);
        AiFeatureOutput.ValidateNextSteps(violations, "nextSteps", new AiRawNextStep?[] { null, new("Open the team", ""), new("Open it", "team") }, 3, bundle);
        AiFeatureOutput.ValidateTexts(violations, "caveats", new string?[] { null, "Fine." }, 3, bundle);

        violations.Select(v => v.Path).ShouldBe(new[]
        {
            "headline", "insights[0]", "insights[1].kind", "insights[1].routeId", "nextSteps[0]", "nextSteps[1].routeId"
        });
        AiFeatureOutput.RenderInsights(null, 5, bundle).ShouldBeEmpty();
        AiFeatureOutput.RenderNextSteps(new AiRawNextStep?[] { new("No route", "") }, 3, bundle).ShouldBeEmpty();
        AiFeatureOutput.RenderFacts(new[] { "{F1}", "F1", "F9", null }, bundle).ShouldHaveSingleItem().Id.ShouldBe("F1");
        AiFeatureOutput.ListValue(bundle, "missing", "key").ShouldBeNull();
        Should.Throw<Volo.Abp.Validation.AbpValidationException>(() => AiFeatureScopeInput.IsTeam("everyone"));
    }

    [Fact]
    public void Every_Data_Quality_Kind_Should_Have_A_Basic_Item()
    {
        var facts = new Dictionary<string, string[]>
        {
            [AiDataQualityKinds.DataGap] = new[] { "first", "last", "weekdays", "threshold" },
            [AiDataQualityKinds.ClockSkew] = new[] { "skew", "threshold" },
            [AiDataQualityKinds.UnmeasuredActivity] = new[] { "share", "unmeasured", "threshold" },
            [AiDataQualityKinds.SoftwareInput] = new[] { "blocks", "threshold" },
            [AiDataQualityKinds.NeverTracked] = new[] { "joined", "window" },
            [AiDataQualityKinds.OutdatedAgent] = new[] { "platform", "version", "newest" }
        };
        var writer = new AiBundleWriter(Caller);
        var handle = writer.AddMember(Ana, "Ana Lima");
        var number = 0;
        foreach (var (kind, keys) in facts)
        {
            var ids = new JsonObject();
            foreach (var key in keys)
            {
                ids[key] = writer.AddFact(key, "value of " + key, handle);
            }

            writer.AddCandidate(new JsonObject
            {
                ["id"] = "C" + ++number,
                ["kind"] = kind,
                ["severity"] = kind is AiDataQualityKinds.ClockSkew or AiDataQualityKinds.DataGap or AiDataQualityKinds.NeverTracked
                    ? AiDataQualitySeverities.Notice
                    : AiDataQualitySeverities.Info,
                ["member"] = handle,
                ["facts"] = ids
            });
        }

        var bundle = writer.Build();
        var feature = new DataQualityReviewFeature(null!, null!, null!);

        var fallback = feature.Fallback(bundle);
        var dto = feature.Render(fallback, bundle, new AiResultMetaDto());

        fallback.Items.Count.ShouldBe(DataQualityReviewFeature.MaxItems);
        dto.Items.Take(3).ShouldAllBe(i => i.Severity == AiDataQualitySeverities.Notice);
        dto.Items.ShouldAllBe(i => i.Explanation.Contains("Ana Lima") && !i.Explanation.Contains('{') && i.PossibleReasons.Count > 0);
        dto.CandidateCount.ShouldBe(6);
        feature.Render(new DataQualityReviewRaw(Array.Empty<DataQualityRawItem?>()), bundle, new AiResultMetaDto()).Items.ShouldBeEmpty();
        feature.NoData(bundle, new AiResultMetaDto()).Items.ShouldBeEmpty();
    }

    [Fact]
    public void The_Creative_Cards_Should_Check_Their_Own_Choices_And_Have_Empty_No_Data_Answers()
    {
        var writer = new AiBundleWriter(Caller);
        writer.SetList(NoteSuggestionsFeature.BlockList, new JsonObject { ["apps"] = new JsonArray(new JsonObject { ["app"] = writer.AddApp("Excel") }) });
        writer.AddTask(Guid.NewGuid(), "Ship it");
        var bundle = writer.Build();
        var notes = new NoteSuggestionsFeature(null!);
        var tasks = new TaskDraftsFeature(null!, null!, null!);

        notes.Validate(new NoteSuggestionsRaw(new NoteSuggestionRaw?[] { null }), bundle).ShouldHaveSingleItem().Path.ShouldBe("suggestions[0]");
        notes.Fallback(bundle).Suggestions.ShouldHaveSingleItem()!.Text.ShouldBe("Worked in {A1}");
        notes.NoData(bundle, new AiResultMetaDto()).Suggestions.ShouldBeEmpty();
        notes.IsRegenerate(new AiNoteSuggestionsInput()).ShouldBeFalse();

        tasks.Validate(new TaskDraftsRaw(new TaskDraftRaw?[] { null, new("Ship the page", "", "Because.", "T9") }), bundle)
            .Select(v => v.Path).ShouldBe(new[] { "drafts[0]", "drafts[1].duplicateOf" });
        tasks.NoData(bundle, new AiResultMetaDto()).Drafts.ShouldBeEmpty();
        tasks.HasFallback.ShouldBeFalse();
        Should.Throw<NotSupportedException>(() => tasks.Fallback(bundle));

        new ReportExplanationFeature(null!, null!, null!).NoData(bundle, new AiResultMetaDto()).Summary
            .ShouldBe(ReportExplanationFeature.NoDataSummary);
    }
}
