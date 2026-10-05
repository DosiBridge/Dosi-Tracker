using Shouldly;
using Xunit;

namespace Dosi.Tracker.Settings;

public class TimeZoneIdsTests
{
    [Theory]
    [InlineData("Asia/Dhaka")]
    [InlineData("America/New_York")]
    [InlineData("Europe/London")]
    [InlineData("UTC")]
    public void An_Iana_Id_Should_Be_Kept_As_Given(string id)
    {
        TimeZoneIds.TryNormalize(id, out var ianaId).ShouldBeTrue();
        ianaId.ShouldBe(id);
    }

    [Fact]
    public void Surrounding_Whitespace_Should_Be_Trimmed()
    {
        TimeZoneIds.TryNormalize("  Asia/Dhaka ", out var ianaId).ShouldBeTrue();
        ianaId.ShouldBe("Asia/Dhaka");
    }

    [Theory]
    [InlineData("Bangladesh Standard Time", "Asia/Dhaka")]
    [InlineData("Tokyo Standard Time", "Asia/Tokyo")]
    public void A_Windows_Id_Should_Be_Converted_To_Iana(string windowsId, string expected)
    {
        TimeZoneIds.TryNormalize(windowsId, out var ianaId).ShouldBeTrue();
        ianaId.ShouldBe(expected);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("Mars/Olympus_Mons")]
    [InlineData("Not a zone")]
    [InlineData("Asia/Dhaka; DROP TABLE")]
    public void An_Unknown_Or_Blank_Id_Should_Be_Rejected(string? id)
    {
        TimeZoneIds.TryNormalize(id, out var ianaId).ShouldBeFalse();
        ianaId.ShouldBeNull();
    }

    [Fact]
    public void An_Id_Longer_Than_The_Limit_Should_Be_Rejected()
    {
        TimeZoneIds.TryNormalize("Asia/" + new string('x', TimeZoneIds.MaxLength), out _).ShouldBeFalse();
    }
}
