using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Teams;

public class InviteUserNamesTests
{
    // ASP.NET Core Identity's default AllowedUserNameCharacters.
    private const string Allowed = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._@+";

    [Theory]
    [InlineData("new.worker@acme.test", true)]
    [InlineData("first+tag@acme.test", true)]
    [InlineData("o'brien@acme.test", false)]
    [InlineData("josé@acme.test", false)]
    [InlineData("a#b@acme.test", false)]
    public void IsAllowed_Should_Follow_The_Identity_Allow_List(string email, bool expected)
    {
        InviteUserNames.IsAllowed(email, Allowed).ShouldBe(expected);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    public void An_Empty_Allow_List_Should_Allow_Everything(string? allowed)
    {
        InviteUserNames.IsAllowed("o'brien@acme.test", allowed).ShouldBeTrue();
    }

    [Theory]
    [InlineData("o'brien@acme.test", "obrien")]
    [InlineData("josé.garcía@acme.test", "jos.garca")]
    [InlineData("'quoted'@acme.test", "quoted")]
    [InlineData("..a&b..@acme.test", "ab")]
    [InlineData("\"weird@local\"@acme.test", "weirdlocal")]
    [InlineData("'''@acme.test", InviteUserNames.FallbackStem)]
    public void Stem_Should_Keep_The_Allowed_Characters_Of_The_Local_Part_Without_At_Signs(string email, string expected)
    {
        var stem = InviteUserNames.Stem(email, Allowed);

        stem.ShouldBe(expected);
        stem.ShouldNotContain("@"); // can never equal somebody's email address
        InviteUserNames.IsAllowed(stem, Allowed).ShouldBeTrue();
    }

    [Fact]
    public void Stem_Should_Leave_Room_For_A_Suffix()
    {
        var stem = InviteUserNames.Stem(new string('a', 300) + "'@acme.test", Allowed);

        stem.Length.ShouldBe(InviteUserNames.MaxStemLength);
        InviteUserNames.Candidate(stem, 99).Length.ShouldBeLessThanOrEqualTo(256);
    }

    [Fact]
    public void Candidates_Should_Be_The_Stem_Then_Numbered()
    {
        InviteUserNames.Candidate("obrien", 1).ShouldBe("obrien");
        InviteUserNames.Candidate("obrien", 2).ShouldBe("obrien2");
        InviteUserNames.Candidate("obrien", 10).ShouldBe("obrien10");
        Should.Throw<ArgumentOutOfRangeException>(() => InviteUserNames.Candidate("obrien", 0));
    }
}
