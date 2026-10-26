using System.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Read;

public class AiActivityParsingTests
{
    private const string Stored =
        """[{"AppName":"chrome.exe","WindowTitle":"Inbox","Seconds":120},{"AppName":"EXCEL.EXE","WindowTitle":"Budget.xlsx","Seconds":300},{"AppName":"Google Chrome","WindowTitle":"Docs","Seconds":240,"KeyboardHits":55}]""";

    [Fact]
    public void Apps_Should_Be_Grouped_By_Canonical_Name_With_The_Most_Seconds_First()
    {
        var usage = AiActivityParsing.Parse(Stored, includeTitles: false);

        usage.Apps.ShouldBe(new[] { new AiAppSeconds("Google Chrome", 360), new AiAppSeconds("Excel", 300) });
        usage.Titles.ShouldBeEmpty();
    }

    [Fact]
    public void Titles_Should_Be_Kept_Only_When_Asked_For()
    {
        var usage = AiActivityParsing.Parse(Stored, includeTitles: true);

        usage.Titles.Select(t => t.Title).ShouldBe(new[] { "Budget.xlsx", "Docs", "Inbox" });
        usage.Titles[0].App.ShouldBe("Excel");
    }

    [Fact]
    public void Older_Entries_Without_Seconds_Should_Keep_Unknown_Seconds()
    {
        var usage = AiActivityParsing.Parse("""[{"AppName":"code","WindowTitle":"x"},{"appName":"slack","seconds":-4},{"AppName":"Code.exe","Seconds":30}]""", false);

        usage.Apps.ShouldBe(new[] { new AiAppSeconds("Visual Studio Code", 30), new AiAppSeconds("Slack", null) });
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("not json")]
    [InlineData("{\"AppName\":\"x\"}")]
    [InlineData("[{\"AppName\":42}]")]
    [InlineData("[null, {\"AppName\":\"\"}, {\"WindowTitle\":\"orphan\"}]")]
    [InlineData("[[[[[[[[[[[[]]]]]]]]]]]]")]
    public void Malformed_Or_Empty_Json_Should_Read_As_Nothing(string? json)
    {
        var usage = AiActivityParsing.Parse(json, includeTitles: true);

        usage.Apps.ShouldBeEmpty();
        usage.Titles.ShouldBeEmpty();
    }

    [Fact]
    public void Oversized_Json_Should_Not_Be_Read()
    {
        var huge = "[" + string.Join(",", Enumerable.Repeat("""{"AppName":"code","Seconds":1}""", 10_000)) + "]";

        AiActivityParsing.Parse(huge, false).Apps.ShouldBeEmpty();
    }
}
