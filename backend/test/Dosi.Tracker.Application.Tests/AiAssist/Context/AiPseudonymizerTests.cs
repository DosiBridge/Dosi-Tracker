using System;
using System.Collections.Generic;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>
/// <see cref="AiPseudonymizer"/>: the reader is always ME, cards number members by tracked minutes then id, a
/// conversation keeps its handles, and no name, user name or email ever reaches the serialized bundle.
/// </summary>
public class AiPseudonymizerTests
{
    private static readonly Guid CallerId = Guid.Parse("00000000-0000-4000-8000-000000000001");
    private static readonly Guid A = Guid.Parse("00000000-0000-4000-8000-00000000000a");
    private static readonly Guid B = Guid.Parse("00000000-0000-4000-8000-00000000000b");
    private static readonly Guid C = Guid.Parse("00000000-0000-4000-8000-00000000000c");
    private static readonly Guid D = Guid.Parse("00000000-0000-4000-8000-00000000000d");

    [Fact]
    public void The_Caller_Should_Be_Me_And_Others_Numbered_On_First_Appearance()
    {
        var pseudonymizer = new AiPseudonymizer(CallerId);

        pseudonymizer.GetOrAdd(CallerId).ShouldBe(AiPseudonymizer.SelfHandle);
        pseudonymizer.GetOrAdd(B).ShouldBe("M1");
        pseudonymizer.GetOrAdd(A).ShouldBe("M2");
        pseudonymizer.GetOrAdd(B).ShouldBe("M1");

        pseudonymizer.Handles.Count.ShouldBe(3);
        pseudonymizer.TryGetHandle(A, out var handle).ShouldBeTrue();
        handle.ShouldBe("M2");
        pseudonymizer.TryGetHandle(C, out _).ShouldBeFalse();

        pseudonymizer.TryResolve("M1", out var resolved).ShouldBeTrue();
        resolved.ShouldBe(B);
        pseudonymizer.TryResolve("ME", out var me).ShouldBeTrue();
        me.ShouldBe(CallerId);
        pseudonymizer.TryResolve("M9", out _).ShouldBeFalse();
    }

    [Fact]
    public void Cards_Should_Number_By_Minutes_Then_Id_Up_To_The_Cap()
    {
        var pseudonymizer = AiPseudonymizer.ForCard(CallerId, new[]
        {
            new AiMemberWeight(C, 30),
            new AiMemberWeight(CallerId, 500), // the caller is ME and takes no number
            new AiMemberWeight(B, 90),
            new AiMemberWeight(A, 90),         // a tie: the lower id first
            new AiMemberWeight(D, 10),
            new AiMemberWeight(D, 25)          // the same member twice: minutes add up (35)
        }, max: 3);

        pseudonymizer.Handles[CallerId].ShouldBe(AiPseudonymizer.SelfHandle);
        pseudonymizer.Handles[A].ShouldBe("M1");
        pseudonymizer.Handles[B].ShouldBe("M2");
        pseudonymizer.Handles[D].ShouldBe("M3");
        pseudonymizer.TryGetHandle(C, out _).ShouldBeFalse(); // beyond the cap: folded into "others"
    }

    [Fact]
    public void Cards_Should_Give_The_Same_Handles_For_The_Same_Data_In_Any_Order()
    {
        var first = AiPseudonymizer.ForCard(CallerId, new[] { new AiMemberWeight(A, 10), new AiMemberWeight(B, 20) });
        var second = AiPseudonymizer.ForCard(CallerId, new[] { new AiMemberWeight(B, 20), new AiMemberWeight(A, 10) });

        second.Handles.ShouldBe(first.Handles);
    }

    [Fact]
    public void A_Conversation_Should_Keep_Its_Handles_And_Number_Newcomers_After_Them()
    {
        var pseudonymizer = AiPseudonymizer.ForConversation(CallerId, new Dictionary<Guid, string>
        {
            [A] = "M4",
            [B] = "M2",
            [D] = "X1",       // malformed: ignored
            [CallerId] = "M7" // the caller is always ME
        });

        pseudonymizer.Handles[A].ShouldBe("M4");
        pseudonymizer.Handles[B].ShouldBe("M2");
        pseudonymizer.Handles[CallerId].ShouldBe(AiPseudonymizer.SelfHandle);
        pseudonymizer.GetOrAdd(D).ShouldBe("M5");
        pseudonymizer.GetOrAdd(C).ShouldBe("M6");

        AiPseudonymizer.ForConversation(CallerId, null).Handles.Count.ShouldBe(1);
    }

    [Fact]
    public void A_Conversation_Should_Give_A_Handle_To_One_Member_Only()
    {
        var pseudonymizer = AiPseudonymizer.ForConversation(CallerId, new Dictionary<Guid, string> { [A] = "M3", [B] = "M3" });

        pseudonymizer.Handles.Values.ShouldBe(new[] { AiPseudonymizer.SelfHandle, "M3" }, ignoreOrder: true);
    }

    [Theory]
    [InlineData("M0")]
    [InlineData("M01")]
    [InlineData("M")]
    [InlineData("m3")]
    [InlineData("M-1")]
    [InlineData("")]
    public void A_Conversation_Should_Ignore_Malformed_Handles(string handle)
    {
        var pseudonymizer = AiPseudonymizer.ForConversation(CallerId, new Dictionary<Guid, string> { [A] = handle });

        pseudonymizer.TryGetHandle(A, out _).ShouldBeFalse();
        pseudonymizer.GetOrAdd(A).ShouldBe("M1");
    }

    [Fact]
    public void A_Serialized_Bundle_Should_Hold_No_Name_User_Name_Or_Email()
    {
        var pseudonymizer = AiPseudonymizer.ForCard(CallerId, new[] { new AiMemberWeight(A, 412) });
        var writer = new AiBundleWriter(pseudonymizer);
        var member = writer.AddMember(A, "Ayesha Rahman");
        writer.AddMember(CallerId, "Caller Person");
        var tracked = writer.AddFact("Tracked time", "6 h 52 min");
        writer.SetList("members", new System.Text.Json.Nodes.JsonArray(
            new System.Text.Json.Nodes.JsonObject { ["member"] = member, ["tracked"] = tracked }));
        // A note that names her is quoted text: its address is redacted, and it is still not her handle's resolution.
        writer.AddNote("Pairing; mail ayesha.rahman@acme.test");

        var bundle = writer.Build();
        var serialized = AiCanonicalJson.Data(bundle) + AiCanonicalJson.Labels(bundle);

        member.ShouldBe("M1");
        serialized.ShouldContain("M1");
        serialized.ShouldNotContain("Ayesha Rahman");
        serialized.ShouldNotContain("Caller Person");
        serialized.ShouldNotContain("ayesha.rahman");
        serialized.ShouldNotContain("@acme.test");
        serialized.ShouldNotContain(A.ToString());
        serialized.ShouldNotContain(CallerId.ToString());
        bundle.Resolution["M1"].DisplayName.ShouldBe("Ayesha Rahman");
    }
}
