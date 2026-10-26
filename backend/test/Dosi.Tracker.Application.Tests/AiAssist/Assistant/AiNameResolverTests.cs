using System;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Read;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>AiNameResolver: diacritics and case are ignored, a first name only counts as a name when it is
/// capitalized, an ambiguous first name gives a clarification instead of a guess, and the caller's own name resolves
/// to {ME}.</summary>
public class AiNameResolverTests
{
    private static readonly Guid CallerId = Guid.NewGuid();
    private static readonly Guid JaneId = Guid.NewGuid();
    private static readonly Guid JohnSmithId = Guid.NewGuid();
    private static readonly Guid JohnDoeId = Guid.NewGuid();

    private static AiRosterEntry Member(Guid id, string name, string surname, string userName) =>
        new(id, userName, name, surname, "member", IsClient: false, DateTime.UtcNow);

    private AiRosterEntry[] Roster() => new[]
    {
        Member(CallerId, "Caller", "Person", "ai.caller"),
        Member(JaneId, "Jané", "Doyle", "jane.doyle"),
        Member(JohnSmithId, "John", "Smith", "john.smith"),
        Member(JohnDoeId, "John", "Doe", "john.doe")
    };

    private static AiPseudonymizer Pseudonymizer() => new(CallerId);

    [Fact]
    public void A_Full_Name_Should_Resolve_Ignoring_Case_And_Diacritics()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("How much did jane DOYLE track?", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldContain("M1");
        pseudonymizer.TryResolve("M1", out var resolved).ShouldBeTrue();
        resolved.ShouldBe(JaneId);
    }

    [Fact]
    public void A_Surname_Alone_Should_Resolve_When_Unique()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("What did Doyle track last week?", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldContain("M1");
    }

    [Fact]
    public void A_Lowercase_First_Name_Should_Not_Be_Treated_As_A_Name()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("did jane finish the report", Roster(), pseudonymizer);

        // "jane" is not capitalized, so it is left exactly as typed.
        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldBe("did jane finish the report");
    }

    [Fact]
    public void An_Ambiguous_First_Name_Should_Give_A_Clarification()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("How much did John track?", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldHaveSingleItem().ShouldBe("John");
        result.RewrittenQuestion.ShouldBe("How much did John track?");
    }

    [Fact]
    public void The_Caller_Should_Resolve_To_Me()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("How much did Caller Person track?", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldContain("ME");
        result.RewrittenQuestion.ShouldNotContain("Caller");
    }

    [Fact]
    public void A_Name_Not_In_The_Roster_Should_Be_Left_As_Typed()
    {
        var pseudonymizer = Pseudonymizer();
        var result = AiNameResolver.Resolve("Did Someone Unknown finish?", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldBe("Did Someone Unknown finish?");
    }

    [Fact]
    public void A_Short_Token_Should_Never_Be_Treated_As_A_Name()
    {
        var pseudonymizer = Pseudonymizer();
        // "Jo" is too short (<3 chars) to match "John", even capitalized.
        var result = AiNameResolver.Resolve("Jo is out today", Roster(), pseudonymizer);

        result.AmbiguousNames.ShouldBeEmpty();
        result.RewrittenQuestion.ShouldBe("Jo is out today");
    }
}
