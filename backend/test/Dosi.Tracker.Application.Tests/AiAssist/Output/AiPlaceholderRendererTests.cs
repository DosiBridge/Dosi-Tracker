using System;
using Dosi.Tracker.AiAssist.Context;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary><see cref="AiPlaceholderRenderer"/>: answers are rendered from the reader's bundle in one pass.</summary>
public class AiPlaceholderRendererTests
{
    private static readonly Guid CallerId = Guid.Parse("00000000-0000-4000-8000-000000000001");
    private static readonly Guid AyeshaId = Guid.Parse("00000000-0000-4000-8000-0000000000a1");
    private static readonly Guid TrickyId = Guid.Parse("00000000-0000-4000-8000-0000000000a2");

    private static AiContextBundle Bundle()
    {
        var writer = new AiBundleWriter(CallerId);
        writer.AddMember(CallerId, "Caller Person");
        writer.AddMember(AyeshaId, "Ayesha Rahman");          // M1
        writer.AddMember(TrickyId, "{F1} $1 {ME}");           // M2: a name is never scanned for placeholders
        writer.AddFact("Tracked time", "6 h 52 min");        // F1
        writer.AddFact("Cost note", "$0 and $1");            // F2: no replacement patterns
        writer.AddProject(Guid.NewGuid(), "Website");         // P1
        writer.AddApp("Visual Studio Code");                  // A1
        writer.AddTitle("Q3 salary review.xlsx");             // W1
        return writer.Build();
    }

    [Fact]
    public void Placeholders_Should_Become_Values_Names_And_Labels()
    {
        AiPlaceholderRenderer.Render("{M1} tracked {F1} on {P1} in {A1} ({W1}).", Bundle())
            .ShouldBe("Ayesha Rahman tracked 6 h 52 min on Website in Visual Studio Code (Q3 salary review.xlsx).");
    }

    [Fact]
    public void The_Reader_Should_Be_You_Capitalized_At_A_Sentence_Or_Line_Start()
    {
        AiPlaceholderRenderer.Render("{ME} tracked {F1}. Then {ME} stopped! {ME} rested? {ME}\n- {ME} worked\n  - {ME} too", Bundle())
            .ShouldBe("You tracked 6 h 52 min. Then you stopped! You rested? You\n- You worked\n  - You too");
        AiPlaceholderRenderer.Render("As {M1} and {ME} did", Bundle()).ShouldBe("As Ayesha Rahman and you did");
        AiPlaceholderRenderer.Render("Shared - {ME} too", Bundle()).ShouldBe("Shared - you too");
    }

    [Fact]
    public void Rendering_Should_Be_A_Single_Pass()
    {
        AiPlaceholderRenderer.Render("{M2} saw {F2}", Bundle()).ShouldBe("{F1} $1 {ME} saw $0 and $1");
    }

    [Fact]
    public void Unknown_Placeholders_Should_Stay_As_They_Are()
    {
        AiPlaceholderRenderer.Render("{F9} and {T1} and {bad}", Bundle()).ShouldBe("{F9} and {T1} and {bad}");
    }

    [Fact]
    public void Empty_Text_Should_Render_Empty()
    {
        AiPlaceholderRenderer.Render(null, Bundle()).ShouldBe("");
        AiPlaceholderRenderer.Render("", Bundle()).ShouldBe("");
    }

    [Fact]
    public void The_Readers_Own_Name_Should_Never_Be_Rendered()
    {
        AiPlaceholderRenderer.Render("{ME} tracked {F1}", Bundle()).ShouldNotContain("Caller Person");
    }
}
