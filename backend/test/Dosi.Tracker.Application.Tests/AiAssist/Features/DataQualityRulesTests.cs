using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>
/// <see cref="DataQualityRules"/>: a hit and a miss for every kind, weekdays counted on the zone's calendar (across a
/// DST change and at the zone's day boundary), only after joining and never today.
/// </summary>
public class DataQualityRulesTests
{
    // Monday 2 March 2026; US daylight saving time starts on Sunday 8 March.
    private static readonly DateTime Monday = new(2026, 3, 2);
    private static readonly Guid Caller = Guid.Parse("00000000-0000-4000-8000-0000000000c0");
    private static readonly Guid Ana = Guid.Parse("00000000-0000-4000-8000-0000000000a1");
    private static readonly Guid Ben = Guid.Parse("00000000-0000-4000-8000-0000000000b2");
    private static readonly Guid Cy = Guid.Parse("00000000-0000-4000-8000-0000000000c3");

    private static DateTime Utc(DateTime local) => DateTime.SpecifyKind(local, DateTimeKind.Utc);

    private static AiRosterEntry Member(Guid id, string role = WorkspaceRoles.Member, bool client = false, DateTime? joined = null) =>
        new(id, "user" + id.ToString("N")[^2..], "Name" + id.ToString("N")[^2..], null, role, client, joined ?? Utc(Monday.AddDays(-60)));

    private static AiMemberDayRow Active(Guid id, DateTime localDate) =>
        new(id, "user", localDate, 1, 60, 0, Utc(localDate.AddHours(9)), Utc(localDate.AddHours(10)));

    private static AiDataQualityRow Quality(Guid id, double tracked = 100, double unmeasured = 0, int? skew = null, int injected = 0, int blocks = 4) =>
        new(id, "user", blocks, tracked, unmeasured, skew, injected, Utc(Monday.AddHours(10)));

    private static AiDeviceRow Device(Guid id, string? platform, string? version, int dayOffset = 0) =>
        new(id, "user", Guid.NewGuid(), platform, version, Utc(Monday.AddDays(dayOffset).AddHours(12)), 60);

    private static DataQualityContext Context(
        IEnumerable<AiMemberDayRow>? days = null,
        IEnumerable<AiDataQualityRow>? quality = null,
        IEnumerable<AiRosterEntry>? roster = null,
        IEnumerable<AiDataQualityRow>? recent = null,
        IEnumerable<AiDeviceRow>? devices = null,
        string zone = "UTC",
        DateTime? fromLocal = null,
        DateTime? toLocal = null,
        DateTime? nowUtc = null,
        bool team = true,
        IReadOnlyCollection<string>? kinds = null)
    {
        var timeZone = TimeZoneInfo.FindSystemTimeZoneById(zone);
        var calendar = new ReportCalendar(timeZone, memberBasis: false);
        return new DataQualityContext(
            timeZone,
            calendar.StartOfDayUtc(fromLocal ?? Monday),
            calendar.StartOfDayUtc(toLocal ?? Monday.AddDays(12)),
            nowUtc ?? Utc(new DateTime(2026, 3, 20, 12, 0, 0)),
            team,
            Caller,
            kinds ?? AiDataQualityKinds.All,
            (roster ?? new[] { Member(Ana), Member(Ben), Member(Cy) }).ToList(),
            (days ?? Array.Empty<AiMemberDayRow>()).ToList(),
            (quality ?? Array.Empty<AiDataQualityRow>()).ToList(),
            (recent ?? new[] { Quality(Ana), Quality(Ben), Quality(Cy) }).ToList(),
            (devices ?? Array.Empty<AiDeviceRow>()).ToList());
    }

    private static Dictionary<string, string> Facts(DataQualityCandidate candidate) =>
        candidate.Facts.ToDictionary(f => f.Key, f => f.Display);

    private static IEnumerable<AiMemberDayRow> ActiveOn(Guid id, params int[] dayOffsets) =>
        dayOffsets.Select(d => Active(id, Monday.AddDays(d)));

    [Fact]
    public void Three_Weekdays_Without_Time_After_Earlier_Time_Should_Be_A_Data_Gap()
    {
        // Ana: Mon, Tue, then nothing Wed to Fri, then the next week.
        var candidates = DataQualityRules.Find(Context(ActiveOn(Ana, 0, 1, 7, 8, 9, 10, 11)));

        var gap = candidates.ShouldHaveSingleItem();
        gap.Kind.ShouldBe(AiDataQualityKinds.DataGap);
        gap.Severity.ShouldBe(AiDataQualitySeverities.Notice);
        gap.UserId.ShouldBe(Ana);
        Facts(gap).ShouldBe(new Dictionary<string, string>
        {
            ["first"] = "Wed 4 Mar", ["last"] = "Fri 6 Mar", ["weekdays"] = "3", ["threshold"] = "3"
        }, ignoreOrder: true);
        gap.Actions.Select(a => (a.Id, a.RouteId)).ShouldBe(new[]
        {
            (DataQualityRules.CheckAgentRunning, (string?)"download"), (DataQualityRules.OpenDownload, (string?)"download")
        });
    }

    [Fact]
    public void Two_Weekdays_Or_A_Weekend_Should_Not_Be_A_Data_Gap()
    {
        // Tue and Wed without time, then Mon 9 alone: never three in a row.
        DataQualityRules.Find(Context(ActiveOn(Ana, 0, 3, 4, 8, 9, 10, 11))).ShouldBeEmpty();
        // Every weekday tracked: the weekends between them never count.
        DataQualityRules.Find(Context(ActiveOn(Ana, 0, 1, 2, 3, 4, 7, 8, 9, 10, 11))).ShouldBeEmpty();
    }

    [Fact]
    public void A_Gap_Should_Run_Across_A_Weekend()
    {
        // Wed, then nothing Thu, Fri, (weekend), Mon.
        var gap = DataQualityRules.Find(Context(ActiveOn(Ana, 2, 8, 9, 10, 11))).ShouldHaveSingleItem();

        Facts(gap)["first"].ShouldBe("Thu 5 Mar");
        Facts(gap)["last"].ShouldBe("Mon 9 Mar");
        Facts(gap)["weekdays"].ShouldBe("3");
    }

    [Fact]
    public void Days_Before_Any_Time_In_The_Range_Should_Not_Count()
    {
        // Ana starts on Thursday of the second week: nothing before it is a gap.
        DataQualityRules.Find(Context(ActiveOn(Ana, 10, 11))).ShouldBeEmpty();
    }

    [Fact]
    public void Weekdays_Should_Be_Counted_On_The_Zones_Calendar_Across_A_Dst_Change()
    {
        // New York, 2 to 11 March (the range ends at the start of Thu 12 March, 04:00 UTC after the change). Ana tracked
        // on Thu 5 March only: Fri 6, Mon 9, Tue 10 and Wed 11 are four weekdays; Thu 12 is outside the range.
        var gap = DataQualityRules.Find(Context(ActiveOn(Ana, 3), zone: "America/New_York", toLocal: Monday.AddDays(10)))
            .ShouldHaveSingleItem();

        Facts(gap)["first"].ShouldBe("Fri 6 Mar");
        Facts(gap)["last"].ShouldBe("Wed 11 Mar");
        Facts(gap)["weekdays"].ShouldBe("4");
    }

    [Fact]
    public void Today_Should_Never_Count_And_Today_Is_The_Zones()
    {
        // Ana tracked on Mon 9 March; the range runs to Sat 14 March. At 20:00 UTC on Thu 12 March it is still Thursday in
        // UTC (Tue and Wed are past: two weekdays) but already Friday in Tokyo (Tue, Wed and Thu are past: three).
        var now = Utc(new DateTime(2026, 3, 12, 20, 0, 0));
        DataQualityRules.Find(Context(ActiveOn(Ana, 7), nowUtc: now)).ShouldBeEmpty();

        var tokyo = DataQualityRules.Find(Context(ActiveOn(Ana, 7), zone: "Asia/Tokyo", nowUtc: now)).ShouldHaveSingleItem();
        Facts(tokyo)["last"].ShouldBe("Thu 12 Mar");
    }

    [Fact]
    public void Only_Weekdays_After_The_Day_A_Member_Joined_Should_Count()
    {
        // Ana tracked on Mon 2 March, left, and joined again on Wed 4 March; the range ends on Fri 6 March. Thu and Fri
        // are two weekdays (Tue to Fri would be four).
        var joined = Member(Ana, joined: Utc(Monday.AddDays(2).AddHours(15)));
        var context = Context(ActiveOn(Ana, 0), roster: new[] { joined }, toLocal: Monday.AddDays(5));

        DataQualityRules.Find(context).ShouldBeEmpty();
        DataQualityRules.Find(context with { Roster = new[] { Member(Ana) } }).ShouldHaveSingleItem().Kind.ShouldBe(AiDataQualityKinds.DataGap);
    }

    [Fact]
    public void A_Member_Who_Is_No_Longer_Active_Should_Not_Be_Reviewed()
    {
        DataQualityRules.Find(Context(ActiveOn(Ana, 0), roster: new[] { Member(Ben) }, recent: new[] { Quality(Ben) })).ShouldBeEmpty();
    }

    [Theory]
    [InlineData(301, true, "5 min")]
    [InlineData(-900, true, "15 min")]
    [InlineData(300, false, null)]
    [InlineData(null, false, null)]
    public void A_Device_Clock_More_Than_Five_Minutes_Off_Should_Be_Flagged(int? skew, bool hit, string? display)
    {
        var candidates = DataQualityRules.Find(Context(quality: new[] { Quality(Ana, skew: skew is { } s ? Math.Abs(s) : null) }));

        if (!hit)
        {
            candidates.ShouldBeEmpty();
            return;
        }

        var candidate = candidates.ShouldHaveSingleItem();
        candidate.Kind.ShouldBe(AiDataQualityKinds.ClockSkew);
        Facts(candidate)["skew"].ShouldBe(display);
        Facts(candidate)["threshold"].ShouldBe("5 min");
        candidate.Actions.ShouldHaveSingleItem().RouteId.ShouldBeNull();
    }

    [Theory]
    [InlineData(31, true)]
    [InlineData(30, false)]
    [InlineData(0, false)]
    public void More_Than_Thirty_Percent_Unmeasured_Should_Be_Flagged(double unmeasured, bool hit)
    {
        var candidates = DataQualityRules.Find(Context(quality: new[] { Quality(Ana, tracked: 100, unmeasured: unmeasured) }));

        candidates.Any(c => c.Kind == AiDataQualityKinds.UnmeasuredActivity).ShouldBe(hit);
        if (hit)
        {
            var candidate = candidates.Single();
            candidate.Severity.ShouldBe(AiDataQualitySeverities.Info);
            Facts(candidate)["share"].ShouldBe("31%");
            Facts(candidate)["threshold"].ShouldBe("30%");
        }
    }

    [Theory]
    [InlineData(3, true)]
    [InlineData(2, false)]
    public void Three_Blocks_With_Software_Input_Should_Be_Flagged(int blocks, bool hit)
    {
        var candidates = DataQualityRules.Find(Context(quality: new[] { Quality(Ana, injected: blocks) }));

        candidates.Any(c => c.Kind == AiDataQualityKinds.SoftwareInput).ShouldBe(hit);
        if (hit)
        {
            candidates.Single().Actions.ShouldBeEmpty();
        }
    }

    [Fact]
    public void A_Member_Who_Never_Tracked_Should_Be_Flagged_For_A_Team_Only()
    {
        var end = Monday.AddDays(12);
        var roster = new[]
        {
            Member(Ana, joined: Utc(end.AddDays(-10))),                         // hit
            Member(Ben, joined: Utc(end.AddDays(-2))),                          // joined too recently
            Member(Cy, client: true, joined: Utc(end.AddDays(-10))),            // a client
            Member(Guid.NewGuid(), WorkspaceRoles.Owner),                       // an owner
            Member(Caller)                                                     // the caller
        };

        var candidate = DataQualityRules.Find(Context(roster: roster, recent: Array.Empty<AiDataQualityRow>())).ShouldHaveSingleItem();
        candidate.Kind.ShouldBe(AiDataQualityKinds.NeverTracked);
        candidate.UserId.ShouldBe(Ana);
        Facts(candidate)["joined"].ShouldBe("Wed 4 Mar 2026");
        Facts(candidate)["window"].ShouldBe("30 days");

        DataQualityRules.Find(Context(roster: roster, recent: new[] { Quality(Ana) })).ShouldBeEmpty();
        DataQualityRules.Find(Context(roster: roster, recent: Array.Empty<AiDataQualityRow>(), team: false)).ShouldBeEmpty();
    }

    [Fact]
    public void An_Agent_Older_Than_The_Newest_On_Its_Platform_Should_Be_Flagged_For_A_Team_Only()
    {
        var devices = new[]
        {
            Device(Ana, "windows", "1.4.0-beta.2"),
            Device(Ben, "windows", "1.10.0"),
            Device(Cy, "windows", "1.3.0", dayOffset: 0), Device(Cy, "windows", "1.10.0", dayOffset: 1), // updated since
            Device(Ana, "macos", "2"),
            Device(Ben, "linux", "nightly"), Device(Cy, "linux", "0.9")                                  // unparsable skipped
        };

        var candidate = DataQualityRules.Find(Context(devices: devices)).ShouldHaveSingleItem();
        candidate.Kind.ShouldBe(AiDataQualityKinds.OutdatedAgent);
        candidate.UserId.ShouldBe(Ana);
        Facts(candidate).ShouldBe(new Dictionary<string, string> { ["platform"] = "windows", ["version"] = "1.4.0", ["newest"] = "1.10.0" },
            ignoreOrder: true);

        DataQualityRules.Find(Context(devices: devices, team: false)).ShouldBeEmpty();
    }

    [Fact]
    public void Only_The_Asked_Kinds_Should_Be_Looked_For_And_Self_Reviews_Never_Get_Team_Kinds()
    {
        var quality = new[] { Quality(Ana, skew: 900, injected: 5) };

        DataQualityRules.Find(Context(quality: quality, kinds: new[] { AiDataQualityKinds.SoftwareInput }))
            .ShouldHaveSingleItem().Kind.ShouldBe(AiDataQualityKinds.SoftwareInput);

        var roster = new[] { Member(Ana), Member(Ben, joined: Utc(Monday)) };
        DataQualityRules.Find(Context(quality: quality, roster: roster, recent: Array.Empty<AiDataQualityRow>(), team: false))
            .Select(c => c.Kind).ShouldBe(new[] { AiDataQualityKinds.ClockSkew, AiDataQualityKinds.SoftwareInput });
    }

    [Fact]
    public void Candidates_Should_Be_Ordered_By_Kind_Then_Tracked_Time_And_Capped()
    {
        var members = Enumerable.Range(1, 30).Select(i => Guid.Parse($"00000000-0000-4000-8000-0000000000{i:00}")).ToList();
        var quality = members.Select((id, i) => Quality(id, tracked: 100 + i, skew: 900, injected: 4)).ToList();

        var candidates = DataQualityRules.Find(Context(quality: quality, roster: members.Select(id => Member(id))));

        candidates.Count.ShouldBe(DataQualityRules.MaxCandidates);
        candidates.ShouldAllBe(c => c.Kind == AiDataQualityKinds.ClockSkew);
        candidates.Select(c => c.Weight).ShouldBeInOrder(SortDirection.Descending);
        candidates[0].DisplayName.ShouldStartWith("Name");
    }

    [Fact]
    public void Every_Kind_Should_Have_Its_Actions_With_Allowlisted_Routes()
    {
        foreach (var kind in AiDataQualityKinds.All)
        {
            DataQualityRules.Actions.ShouldContainKey(kind);
            DataQualityRules.Actions[kind].Where(a => a.RouteId != null).ShouldAllBe(a => Output.AiRouteIds.IsAllowed(a.RouteId));
        }

        DataQualityRules.SeverityRank(AiDataQualitySeverities.Notice).ShouldBeLessThan(DataQualityRules.SeverityRank(AiDataQualitySeverities.Info));
    }
}
