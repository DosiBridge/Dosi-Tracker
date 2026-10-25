using Shouldly;
using Xunit;

namespace Dosi.Tracker.Erasure;

/// <summary>A deleted account's timelines keep their minutes and counts but lose every application name.</summary>
public class AccountEraserTimelineTests
{
    [Fact]
    public void Application_Names_Are_Removed_And_Everything_Else_Kept()
    {
        var scrubbed = AccountEraser.ScrubTimeline(
            "[{\"Minute\":0,\"KeyboardHits\":12,\"Active\":true,\"AppName\":\"KeePass\",\"ActiveSeconds\":55}," +
            "{\"Minute\":1,\"Active\":false,\"appName\":\"bank.example\"}]");

        scrubbed.ShouldBe("[{\"Minute\":0,\"KeyboardHits\":12,\"Active\":true,\"ActiveSeconds\":55},{\"Minute\":1,\"Active\":false}]");
        scrubbed.ShouldNotContain("ppName");
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("not json")]
    [InlineData("{\"Minute\":0}")]
    public void An_Unreadable_Timeline_Becomes_Empty(string? timeline)
    {
        AccountEraser.ScrubTimeline(timeline).ShouldBe("[]");
    }

    [Fact]
    public void The_Placeholder_Name_Identifies_Nobody()
    {
        var userId = System.Guid.Parse("4f6e1c1e-2d1b-4d55-9a3b-3b0d6d8f0a11");

        AccountEraser.PlaceholderUserName(userId).ShouldBe("deleted-4f6e1c1e2d1b4d559a3b3b0d6d8f0a11");
    }
}
