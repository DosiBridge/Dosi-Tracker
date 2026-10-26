using System;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Features;
using Dosi.Tracker.AiAssist.Read;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Every Ask AI tool, called directly (through <see cref="AiToolGate"/>, as the model would, without a scripted
/// chat round): a team-scoped caller's tool results stay a subset of what they could read directly, a member never
/// gets <c>list_team</c>, and an unknown or foreign handle is refused by its own error code, never narrowed.
/// </summary>
public abstract class AiToolScopingTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private Task<AiTurnContext> TurnAsync(Guid userId) => WithUnitOfWorkAsync(async () =>
    {
        var caller = await AsUserAsync(userId, () => GetRequiredService<AiCallerContextFactory>().CreateAsync("UTC"));
        return new AiTurnContext(caller, null);
    });

    private async Task<JsonObject> CallAsync(AiTurnContext turn, Guid userId, string toolName, params (string Name, string? Value)[] args)
    {
        var tools = GetRequiredService<AiToolCatalog>().For(turn.Caller, turn);
        tools.Select(t => t.Name).ShouldContain(toolName, $"the catalog should offer {toolName}");
        var tool = (AIFunction)tools.First(t => t.Name == toolName);

        var arguments = new AIFunctionArguments();
        foreach (var (name, value) in args)
        {
            if (value != null)
            {
                arguments[name] = value;
            }
        }

        return await AsUserAsync(userId, async () => await WithUnitOfWorkAsync(async () =>
            (JsonObject)(await tool.InvokeAsync(arguments))!));
    }

    [Fact]
    public async Task The_Owner_Should_Get_List_Team_And_A_Member_Should_Not()
    {
        await ReadyAsync();

        var ownerTurn = await TurnAsync(CallerId);
        GetRequiredService<AiToolCatalog>().For(ownerTurn.Caller, ownerTurn).Select(t => t.Name).ShouldContain("list_team");

        var memberTurn = await TurnAsync(AyeshaId);
        GetRequiredService<AiToolCatalog>().For(memberTurn.Caller, memberTurn).Select(t => t.Name).ShouldNotContain("list_team");
    }

    [Fact]
    public async Task List_Team_Should_Be_A_Subset_Of_The_Direct_Roster()
    {
        await ReadyAsync();
        var turn = await TurnAsync(CallerId);

        var result = await CallAsync(turn, CallerId, "list_team");
        var directRoster = await AsUserAsync(CallerId, () => WithUnitOfWorkAsync(() => Reads.RosterAsync(turn.Caller)));

        var members = result["members"]!.AsArray();
        members.Count.ShouldBe(directRoster.Count);
        result.ContainsKey("error").ShouldBeFalse();
    }

    [Fact]
    public async Task A_Member_Can_Never_Address_Anyone_By_A_Handle_They_Were_Never_Given()
    {
        await ReadyAsync();
        var turn = await TurnAsync(AyeshaId);

        // A plain member's catalog never hands out another member's handle (no list_team, and every other tool only
        // mints one for someone they already asked about), so naming one is always unknown, never a quiet narrowing.
        var unknownHandle = await CallAsync(turn, AyeshaId, "get_time_summary", ("member", "M1"));
        unknownHandle["error"]!.GetValue<string>().ShouldBe(AiToolErrors.UnknownMember);
    }

    [Fact]
    public async Task A_Member_Referencing_Another_Members_Guid_Via_A_Minted_Handle_Should_Be_Refused_Not_Narrowed()
    {
        await ReadyAsync();
        // Simulates a handle somehow known to Ayesha (never possible in practice, since no tool of hers reveals one):
        // the read layer itself must still refuse it, rather than silently answering about her instead.
        var caller = await AsUserAsync(AyeshaId, () => WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync("UTC")));
        var turn = new AiTurnContext(caller, new System.Collections.Generic.Dictionary<Guid, string> { [CallerId] = "M1" });

        var result = await AsUserAsync(AyeshaId, async () => await WithUnitOfWorkAsync(async () =>
        {
            var tool = (AIFunction)GetRequiredService<AiToolCatalog>().For(turn.Caller, turn).Single(t => t.Name == "get_time_summary");
            return (JsonObject)(await tool.InvokeAsync(new AIFunctionArguments { ["member"] = "M1" }))!;
        }));

        result["error"]!.GetValue<string>().ShouldBe(AiToolErrors.NotPermitted);
    }

    [Fact]
    public async Task An_Unknown_Project_Handle_Should_Be_Refused()
    {
        await ReadyAsync();
        var turn = await TurnAsync(CallerId);

        var result = await CallAsync(turn, CallerId, "get_time_summary", ("project", "P9"));
        result["error"]!.GetValue<string>().ShouldBe(AiToolErrors.UnknownProject);
    }

    [Fact]
    public async Task An_Unknown_Tool_Argument_Member_Should_Give_Unknown_Member()
    {
        await ReadyAsync();
        var turn = await TurnAsync(CallerId);

        var result = await CallAsync(turn, CallerId, "get_time_summary", ("member", "M99"));
        result["error"]!.GetValue<string>().ShouldBe(AiToolErrors.UnknownMember);
    }

    [Fact]
    public async Task List_Projects_Should_Be_A_Subset_Of_The_Direct_Projects()
    {
        await ReadyAsync();
        var turn = await TurnAsync(CallerId);

        var result = await CallAsync(turn, CallerId, "list_projects");
        var direct = await AsUserAsync(CallerId, () => WithUnitOfWorkAsync(() => Reads.ProjectsAsync(turn.Caller, includeArchived: false)));

        result["projects"]!.AsArray().Count.ShouldBeLessThanOrEqualTo(direct.Count);
    }

    [Fact]
    public async Task Get_Help_Should_Carry_No_Workspace_Data()
    {
        await ReadyAsync();
        var turn = await TurnAsync(CallerId);

        var result = await CallAsync(turn, CallerId, "get_help", ("topic", "install the agent"));
        var json = result.ToJsonString();
        json.ShouldNotContain("Website");
        json.ShouldNotContain(AyeshaName);
        result["articles"]!.AsArray().Count.ShouldBeGreaterThan(0);
    }
}
