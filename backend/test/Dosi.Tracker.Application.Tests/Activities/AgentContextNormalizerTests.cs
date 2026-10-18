using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Activities;

/* Pure unit tests for the normalization of the optional agent context (shared contract §1, §3).
 * These do not need the ABP test module infrastructure. */
public class AgentContextNormalizerTests
{
    [Theory]
    [InlineData(null, null)]
    [InlineData("", null)]
    [InlineData("   ", null)]
    [InlineData("\u0001\u0007", null)]
    [InlineData("  DESKTOP-42  ", "DESKTOP-42")]
    [InlineData("DESK\u0000TOP\n-42", "DESKTOP-42")]
    public void CleanText_Should_Strip_Control_Characters_And_Surrounding_Spaces(string? value, string? expected)
    {
        AgentContextNormalizer.CleanText(value, 64).ShouldBe(expected);
    }

    [Fact]
    public void CleanText_Should_Cut_To_The_Limit_Without_Trailing_Spaces()
    {
        AgentContextNormalizer.CleanText("abcde fghij", 6).ShouldBe("abcde");
        AgentContextNormalizer.CleanText(new string('x', 200), 32).ShouldBe(new string('x', 32));
    }

    [Fact]
    public void Truncate_Should_Never_Split_A_Surrogate_Pair()
    {
        // "ab" + U+1F600 (two UTF-16 units): cutting after 3 units would leave half of the pair.
        var text = "ab\U0001F600cd";

        AgentContextNormalizer.Truncate(text, 3).ShouldBe("ab");
        AgentContextNormalizer.Truncate(text, 4).ShouldBe("ab\U0001F600");
        AgentContextNormalizer.Truncate("short", 10).ShouldBe("short");
        AgentContextNormalizer.Truncate(null, 10).ShouldBeNull();
    }

    [Theory]
    [InlineData("  Fixing\tthe   login\r\nbug \u0007 ", "Fixing the login bug")]
    [InlineData("a\u0007b", "ab")]
    [InlineData("a \u0007 b", "a b")]
    [InlineData(" \t\r\n ", null)]
    [InlineData(null, null)]
    public void CleanNote_Should_Collapse_Whitespace_And_Drop_Control_Characters(string? value, string? expected)
    {
        AgentContextNormalizer.CleanNote(value, 2048).ShouldBe(expected);
    }

    [Fact]
    public void CleanNote_Should_Cut_To_The_Limit()
    {
        AgentContextNormalizer.CleanNote("one two three", 7).ShouldBe("one two");
        AgentContextNormalizer.CleanNote("one two three", 8).ShouldBe("one two");
    }

    [Theory]
    [InlineData("windows", "windows")]
    [InlineData(" MacOS ", "macos")]
    [InlineData("linux", "linux")]
    [InlineData("freebsd", null)]
    [InlineData("", null)]
    [InlineData(null, null)]
    public void NormalizePlatform_Should_Keep_Only_Known_Platforms(string? value, string? expected)
    {
        AgentContextNormalizer.NormalizePlatform(value).ShouldBe(expected);
    }

    [Theory]
    [InlineData("interval", "interval")]
    [InlineData("SLEEP", "sleep")]
    [InlineData(" shutdown ", "shutdown")]
    [InlineData("crash", null)]
    [InlineData(null, null)]
    public void NormalizeEndReason_Should_Keep_Only_Known_Reasons(string? value, string? expected)
    {
        AgentContextNormalizer.NormalizeEndReason(value).ShouldBe(expected);
    }

    [Fact]
    public void NormalizeEndReason_Should_Accept_Every_Reason_Of_The_Contract()
    {
        foreach (var reason in new[] { "interval", "pause", "break", "idle", "lock", "sleep", "project", "task", "signout", "quit", "shutdown" })
        {
            AgentContextNormalizer.NormalizeEndReason(reason).ShouldBe(reason);
        }
    }

    [Theory]
    [InlineData(0, 0)]
    [InlineData(360, 360)]
    [InlineData(-840, -840)]
    [InlineData(840, 840)]
    [InlineData(841, null)]
    [InlineData(-841, null)]
    [InlineData(null, null)]
    public void NormalizeUtcOffset_Should_Keep_Offsets_Between_Minus_And_Plus_14_Hours(int? minutes, int? expected)
    {
        AgentContextNormalizer.NormalizeUtcOffset(minutes).ShouldBe(expected);
    }

    [Fact]
    public void NormalizeDeviceId_Should_Treat_The_Empty_Guid_As_Not_Sent()
    {
        var id = Guid.NewGuid();

        AgentContextNormalizer.NormalizeDeviceId(id).ShouldBe(id);
        AgentContextNormalizer.NormalizeDeviceId(Guid.Empty).ShouldBeNull();
        AgentContextNormalizer.NormalizeDeviceId(null).ShouldBeNull();
    }

    [Theory]
    [InlineData("Asia/Dhaka", "Asia/Dhaka")]
    [InlineData(" Europe/London ", "Europe/London")]
    [InlineData("UTC", "UTC")]
    [InlineData("Mars/Olympus_Mons", null)]
    [InlineData("", null)]
    [InlineData(null, null)]
    public void NormalizeTimeZone_Should_Keep_Known_Zones(string? value, string? expected)
    {
        AgentContextNormalizer.NormalizeTimeZone(value).ShouldBe(expected);
    }

    [Fact]
    public void NormalizeTimeZone_Should_Store_A_Windows_Zone_As_Its_Iana_Id()
    {
        AgentContextNormalizer.NormalizeTimeZone("Bangladesh Standard Time").ShouldBe("Asia/Dhaka");
    }

    [Fact]
    public void NormalizeTimeZone_Should_Refuse_An_Over_Long_Id()
    {
        AgentContextNormalizer.NormalizeTimeZone("Asia/" + new string('x', ActivityConsts.MaxTimeZoneLength)).ShouldBeNull();
    }

    [Fact]
    public void ClockSkewSeconds_Should_Be_Server_Time_Minus_Device_Time()
    {
        var serverNow = new DateTime(2026, 10, 5, 9, 0, 0, DateTimeKind.Utc);

        AgentContextNormalizer.ClockSkewSeconds(serverNow, serverNow.AddSeconds(-42)).ShouldBe(42);  // device behind
        AgentContextNormalizer.ClockSkewSeconds(serverNow, serverNow.AddSeconds(17)).ShouldBe(-17);  // device ahead
        AgentContextNormalizer.ClockSkewSeconds(serverNow, serverNow.AddMilliseconds(-1500)).ShouldBe(2); // rounded half away from zero
        AgentContextNormalizer.ClockSkewSeconds(serverNow, null).ShouldBeNull();
    }

    [Fact]
    public void ClockSkewSeconds_Should_Be_Clamped_To_Ten_Days()
    {
        var serverNow = new DateTime(2026, 10, 5, 9, 0, 0, DateTimeKind.Utc);

        AgentContextNormalizer.ClockSkewSeconds(serverNow, serverNow.AddYears(-3)).ShouldBe(AgentContextNormalizer.MaxClockSkewSeconds);
        AgentContextNormalizer.ClockSkewSeconds(serverNow, serverNow.AddDays(11)).ShouldBe(-AgentContextNormalizer.MaxClockSkewSeconds);
        AgentContextNormalizer.MaxClockSkewSeconds.ShouldBe(864000);
    }

    [Fact]
    public void ClockSkewSeconds_Should_Read_An_Unspecified_Time_As_Utc()
    {
        var serverNow = new DateTime(2026, 10, 5, 9, 0, 0, DateTimeKind.Utc);
        var sentAt = DateTime.SpecifyKind(serverNow.AddSeconds(-30), DateTimeKind.Unspecified);

        AgentContextNormalizer.ClockSkewSeconds(serverNow, sentAt).ShouldBe(30);
    }
}
