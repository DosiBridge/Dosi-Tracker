using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Platform;

/// <summary>AI assist in the platform console. Requires <see cref="TrackerPermissions.Platform.Default"/> (the global
/// platform-admin role only) and works across every workspace.</summary>
[Authorize(TrackerPermissions.Platform.Default)]
[WorkspaceNotRequired]
public class PlatformAiAppService : TrackerAppService, IPlatformAiAppService
{
    /// <summary>How many workspaces the overview lists.</summary>
    public const int TopWorkspaceCount = 10;

    private readonly IAiChatClientProvider _provider;
    private readonly IOptions<AiOptions> _options;
    private readonly AiSettingsManager _aiSettings;
    private readonly AiUsageLedger _ledger;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly AiCircuitBreaker _circuitBreaker;

    public PlatformAiAppService(
        IAiChatClientProvider provider,
        IOptions<AiOptions> options,
        AiSettingsManager aiSettings,
        AiUsageLedger ledger,
        IRepository<Workspace, Guid> workspaceRepository,
        AiCircuitBreaker circuitBreaker)
    {
        _provider = provider;
        _options = options;
        _aiSettings = aiSettings;
        _ledger = ledger;
        _workspaceRepository = workspaceRepository;
        _circuitBreaker = circuitBreaker;
    }

    public virtual async Task<PlatformAiOverviewDto> GetOverviewAsync()
    {
        var options = _options.Value;
        var monthStart = AiUsageRecordQueries.MonthStart(Clock.Now);
        var usage = await _ledger.GetPlatformAsync(monthStart);
        var configured = _provider.State == AiProviderState.Configured;

        var requests = usage.Sum(u => u.Requests);
        var costs = usage.Where(u => u.EstimatedCostUsd.HasValue).Select(u => u.EstimatedCostUsd!.Value).ToList();

        var top = usage
            .OrderByDescending(u => u.InputTokens + u.OutputTokens)
            .ThenByDescending(u => u.Requests)
            .ThenBy(u => u.WorkspaceId)
            .Take(TopWorkspaceCount)
            .ToList();
        var topIds = top.Select(u => u.WorkspaceId).ToList();
        // Deleted workspaces are filtered out here, so their rows show the placeholder name.
        var names = (await _workspaceRepository.GetListAsync(w => topIds.Contains(w.Id))).ToDictionary(w => w.Id, w => w.Name);

        return new PlatformAiOverviewDto
        {
            ProviderState = ToWire(_provider.State),
            ProviderDisplayName = configured ? _provider.Descriptor?.DisplayName : null,
            DefaultModel = configured ? _provider.Descriptor?.DefaultModel : null,
            ConfigEnabled = options.Enabled,
            RuntimeEnabled = await _aiSettings.IsPlatformEnabledAsync(),
            // This instance's breaker: with several API instances, each pauses on its own.
            BreakerState = _circuitBreaker.StateName,
            MonthStart = monthStart,
            Requests = requests,
            InputTokens = usage.Sum(u => u.InputTokens),
            OutputTokens = usage.Sum(u => u.OutputTokens),
            EstimatedCostUsd = costs.Count == 0 ? null : costs.Sum(),
            BudgetTokens = options.Budget.MonthlyTokenLimit,
            BudgetUsd = options.Budget.MonthlyUsdLimit,
            FallbackRate = requests == 0 ? 0d : (double)usage.Sum(u => u.Fallbacks) / requests,
            ErrorRate = requests == 0 ? 0d : (double)usage.Sum(u => u.Errors) / requests,
            TopWorkspaces = top
                .Select(u => new PlatformAiWorkspaceUsageDto
                {
                    WorkspaceId = u.WorkspaceId,
                    WorkspaceName = names.TryGetValue(u.WorkspaceId, out var name) ? name : PlatformAppService.DeletedWorkspaceName,
                    Requests = u.Requests,
                    Tokens = u.InputTokens + u.OutputTokens,
                    EstimatedCostUsd = u.EstimatedCostUsd
                })
                .ToList()
        };
    }

    public virtual async Task<PlatformAiStatusDto> UpdateStatusAsync(PlatformAiStatusDto input)
    {
        Check.NotNull(input, nameof(input));
        await _aiSettings.SetPlatformEnabledAsync(input.Enabled);
        return new PlatformAiStatusDto { Enabled = await _aiSettings.IsPlatformEnabledAsync() };
    }

    /// <summary>The wire form of a provider state ("missing-api-key").</summary>
    public static string ToWire(AiProviderState state) => state switch
    {
        AiProviderState.Configured => "configured",
        AiProviderState.Disabled => "disabled",
        AiProviderState.MissingApiKey => "missing-api-key",
        AiProviderState.MissingModel => "missing-model",
        AiProviderState.InvalidEndpoint => "invalid-endpoint",
        AiProviderState.UnsupportedProvider => "unsupported-provider",
        _ => "disabled"
    };
}
