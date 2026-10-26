using System.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>The feature list and data categories are wire values: fixed ids, environment-friendly config keys.</summary>
public class AiFeatureIdsTests
{
    [Fact]
    public void Every_Feature_Has_A_Unique_Id_And_An_Environment_Friendly_Config_Key()
    {
        AiFeatureIds.All.Select(f => f.Id).ShouldBe(new[]
        {
            "period-summary", "member-day-summary", "report-explanation", "data-quality-review", "note-suggestions",
            "task-drafts", "ask"
        });
        AiFeatureIds.All.Select(f => f.ConfigKey).ShouldBeUnique();
        // "-" and "." cannot be set from a shell as part of an environment variable name.
        AiFeatureIds.All.ShouldAllBe(f => f.ConfigKey.All(char.IsLetterOrDigit));
        AiFeatureIds.All.ShouldAllBe(f => f.DefaultMaxOutputTokens > 0);
    }

    [Theory]
    [InlineData("ask", true)]
    [InlineData("task-drafts", true)]
    [InlineData("Ask", false)]
    [InlineData("TaskDrafts", false)]
    [InlineData("", false)]
    [InlineData(null, false)]
    public void Only_Listed_Ids_Are_Known(string? id, bool known)
    {
        AiFeatureIds.IsKnown(id).ShouldBe(known);
        (AiFeatureIds.Find(id) != null).ShouldBe(known);
    }

    [Fact]
    public void Data_Categories_Are_Fixed_And_Never_Include_Captures_Or_Input()
    {
        AiDataCategories.All.Count.ShouldBe(10);
        AiDataCategories.All.ShouldBeUnique();
        AiDataCategories.IsKnown(AiDataCategories.WindowTitles).ShouldBeTrue();
        AiDataCategories.IsKnown("screenshots").ShouldBeFalse();
        AiDataCategories.IsKnown("keystrokes").ShouldBeFalse();
        AiDataCategories.IsKnown(null).ShouldBeFalse();
    }
}
