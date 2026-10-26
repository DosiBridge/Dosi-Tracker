using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Platform;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>AI assist in the platform console: the provider's state, the month's usage across workspaces, and the
/// runtime switch, which applies to the next request.</summary>
public abstract class PlatformAiAppServiceTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IPlatformAiAppService _service;

    protected PlatformAiAppServiceTests()
    {
        _service = GetRequiredService<IPlatformAiAppService>();
    }

    [Fact]
    public async Task An_Unused_Platform_Should_Report_The_Provider_And_Zeros()
    {
        var overview = await _service.GetOverviewAsync();

        overview.ProviderState.ShouldBe("configured");
        overview.ProviderDisplayName.ShouldBe("Fake");
        overview.DefaultModel.ShouldBe(Fakes.FakeChatClient.DefaultModel);
        overview.ConfigEnabled.ShouldBeFalse();
        overview.RuntimeEnabled.ShouldBeTrue();
        overview.BreakerState.ShouldBe("closed");
        overview.MonthStart.ShouldBe(AiUsageRecordQueries.MonthStart(Now));
        overview.Requests.ShouldBe(0);
        overview.InputTokens.ShouldBe(0);
        overview.OutputTokens.ShouldBe(0);
        overview.EstimatedCostUsd.ShouldBeNull();
        overview.BudgetTokens.ShouldBe(20_000_000);
        overview.BudgetUsd.ShouldBeNull();
        overview.FallbackRate.ShouldBe(0);
        overview.ErrorRate.ShouldBe(0);
        overview.TopWorkspaces.ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Overview_Should_Show_The_Instances_Circuit_Breaker()
    {
        GetRequiredService<AiCircuitBreaker>().RecordFailure(AiProviderErrorKind.Auth);

        (await _service.GetOverviewAsync()).BreakerState.ShouldBe("open");
    }

    [Fact]
    public async Task The_Overview_Should_Sum_The_Month_Across_Workspaces_And_Rank_Them_By_Tokens()
    {
        var big = await ServiceProvider.CreateWorkspaceAsync("Big Co");
        var small = await ServiceProvider.CreateWorkspaceAsync("Small Co");
        var gone = await ServiceProvider.CreateWorkspaceAsync("Gone Co");
        var user = Guid.NewGuid();

        await InsertUsageAsync(user, inputTokens: 5000, outputTokens: 1000, cost: 0.01m, workspaceId: big);
        await InsertUsageAsync(user, outcome: AiUsageOutcomes.Fallback, inputTokens: 1000, outputTokens: 0, workspaceId: big);
        await InsertUsageAsync(user, outcome: AiUsageOutcomes.ProviderError, inputTokens: null, outputTokens: null, workspaceId: small);
        await InsertUsageAsync(user, outcome: AiUsageOutcomes.Timeout, inputTokens: 10, outputTokens: 0, cost: 0.002m, workspaceId: small);
        await InsertUsageAsync(user, inputTokens: 300, outputTokens: 30, workspaceId: gone);
        // Not counted: no provider call, or last month.
        await InsertUsageAsync(user, outcome: AiUsageOutcomes.Cached, cacheHit: true, inputTokens: 100_000, workspaceId: small);
        await InsertUsageAsync(user, at: AiUsageRecordQueries.MonthStart(Now).AddMinutes(-1), inputTokens: 100_000, workspaceId: small);
        await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Workspace, Guid>>().DeleteAsync(gone));

        var overview = await _service.GetOverviewAsync();

        overview.Requests.ShouldBe(5);
        overview.InputTokens.ShouldBe(5000 + 1000 + 10 + 300);
        overview.OutputTokens.ShouldBe(1000 + 30);
        overview.EstimatedCostUsd.ShouldBe(0.012m);
        overview.FallbackRate.ShouldBe(0.2, tolerance: 1e-9);
        overview.ErrorRate.ShouldBe(0.4, tolerance: 1e-9);
        overview.TopWorkspaces.Select(w => (w.WorkspaceId, w.WorkspaceName, w.Requests, w.Tokens)).ShouldBe(new[]
        {
            (big, "Big Co", 2L, 7000L),
            (gone, PlatformAppService.DeletedWorkspaceName, 1L, 330L),
            (small, "Small Co", 2L, 10L)
        });
        overview.TopWorkspaces[0].EstimatedCostUsd.ShouldBe(0.01m);
        overview.TopWorkspaces[1].EstimatedCostUsd.ShouldBeNull();
    }

    [Fact]
    public async Task At_Most_Ten_Workspaces_Should_Be_Listed()
    {
        for (var i = 0; i < PlatformAiAppService.TopWorkspaceCount + 2; i++)
        {
            await InsertUsageAsync(Guid.NewGuid(), inputTokens: 100 + i, workspaceId: Guid.NewGuid());
        }

        var overview = await _service.GetOverviewAsync();

        overview.Requests.ShouldBe(PlatformAiAppService.TopWorkspaceCount + 2);
        overview.TopWorkspaces.Count.ShouldBe(PlatformAiAppService.TopWorkspaceCount);
        overview.TopWorkspaces.Select(w => w.Tokens).ShouldBeInOrder(SortDirection.Descending);
    }

    [Fact]
    public async Task The_Runtime_Switch_Should_Apply_To_The_Next_Request()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        var guard = GetRequiredService<AiAccessGuard>();
        var caller = await WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync());

        (await _service.UpdateStatusAsync(new PlatformAiStatusDto { Enabled = false })).Enabled.ShouldBeFalse();

        (await _service.GetOverviewAsync()).RuntimeEnabled.ShouldBeFalse();
        (await GetRequiredService<IAiAssistAppService>().GetStatusAsync()).Reason.ShouldBe(AiStatusReasons.PlatformDisabled);
        (await Should.ThrowAsync<BusinessException>(() => WithUnitOfWorkAsync(() => guard.EnsureAsync(caller, AiFeatureIds.AskId))))
            .Code.ShouldBe(TrackerDomainErrorCodes.AiPlatformDisabled);

        (await _service.UpdateStatusAsync(new PlatformAiStatusDto { Enabled = true })).Enabled.ShouldBeTrue();
        await WithUnitOfWorkAsync(() => guard.EnsureAsync(caller, AiFeatureIds.AskId));
        Provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Unconfigured_Provider_Should_Show_Its_State_And_Nothing_Else()
    {
        Provider.State = AiProviderState.MissingApiKey;

        var overview = await _service.GetOverviewAsync();

        overview.ProviderState.ShouldBe("missing-api-key");
        overview.ProviderDisplayName.ShouldBeNull();
        overview.DefaultModel.ShouldBeNull();
    }

    [Theory]
    [InlineData(AiProviderState.Configured, "configured")]
    [InlineData(AiProviderState.Disabled, "disabled")]
    [InlineData(AiProviderState.MissingApiKey, "missing-api-key")]
    [InlineData(AiProviderState.MissingModel, "missing-model")]
    [InlineData(AiProviderState.InvalidEndpoint, "invalid-endpoint")]
    [InlineData(AiProviderState.UnsupportedProvider, "unsupported-provider")]
    public void Provider_States_Should_Have_Their_Wire_Names(AiProviderState state, string wire)
    {
        PlatformAiAppService.ToWire(state).ShouldBe(wire);
    }

    [Fact]
    public void An_Unknown_Provider_State_Should_Read_As_Disabled()
    {
        PlatformAiAppService.ToWire((AiProviderState)99).ShouldBe("disabled");
    }
}
