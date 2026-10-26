using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Options;
using Volo.Abp.Authorization;
using Volo.Abp.Identity;
using Volo.Abp.Users;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI assist in the current workspace (<c>X-Workspace-Id</c> is required: workspace resolution has already refused a
/// caller who is not an active member). This part holds the status every member reads, the owner's settings and the usage
/// views; nothing here calls a provider.
/// </summary>
[Authorize]
public partial class AiAssistAppService : TrackerAppService, IAiAssistAppService
{
    /// <summary>en.json keys of the disclosure lists, in display order.</summary>
    public static readonly string[] AlwaysSentKeys =
    {
        "AiAssist:Disclosure:AlwaysSent:Durations",
        "AiAssist:Disclosure:AlwaysSent:ProjectsAndTasks",
        "AiAssist:Disclosure:AlwaysSent:Apps",
        "AiAssist:Disclosure:AlwaysSent:Agent",
        "AiAssist:Disclosure:AlwaysSent:Questions"
    };

    public static readonly string[] SentWhenAllowedKeys =
    {
        "AiAssist:Disclosure:SentWhenAllowed:WindowTitles",
        "AiAssist:Disclosure:SentWhenAllowed:Notes"
    };

    public static readonly string[] NeverSentKeys =
    {
        "AiAssist:Disclosure:NeverSent:Captures",
        "AiAssist:Disclosure:NeverSent:Keystrokes",
        "AiAssist:Disclosure:NeverSent:Identities",
        "AiAssist:Disclosure:NeverSent:Devices",
        "AiAssist:Disclosure:NeverSent:Pay",
        "AiAssist:Disclosure:NeverSent:WorkspaceName"
    };

    public static readonly string[] WhoCanUseKeys =
    {
        "AiAssist:Disclosure:WhoCanUse:Owners",
        "AiAssist:Disclosure:WhoCanUse:Managers",
        "AiAssist:Disclosure:WhoCanUse:Members",
        "AiAssist:Disclosure:WhoCanUse:Clients"
    };

    public const string EnabledNotificationKey = "AiAssist:Notification:Enabled";
    public const string MoreDataNotificationKey = "AiAssist:Notification:MoreData";
    public const string AlsoIncludesNotificationKey = "AiAssist:Notification:AlsoIncludes";
    public const string WindowTitlesCategoryKey = "AiAssist:Notification:WindowTitles";
    public const string NotesCategoryKey = "AiAssist:Notification:Notes";
    public const string WindowTitlesAndNotesCategoryKey = "AiAssist:Notification:WindowTitlesAndNotes";

    private readonly AiCallerContextFactory _callerContextFactory;
    private readonly AiAccessGuard _accessGuard;
    private readonly AiSettingsManager _aiSettings;
    private readonly AiUsageLedger _ledger;
    private readonly WorkspaceMemberNotifier _memberNotifier;
    private readonly WorkspaceMembershipManager _memberships;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IAiChatClientProvider _provider;
    private readonly IOptions<AiOptions> _options;

    public AiAssistAppService(
        AiCallerContextFactory callerContextFactory,
        AiAccessGuard accessGuard,
        AiSettingsManager aiSettings,
        AiUsageLedger ledger,
        WorkspaceMemberNotifier memberNotifier,
        WorkspaceMembershipManager memberships,
        IIdentityUserRepository userRepository,
        IAiChatClientProvider provider,
        IOptions<AiOptions> options)
    {
        _callerContextFactory = callerContextFactory;
        _accessGuard = accessGuard;
        _aiSettings = aiSettings;
        _ledger = ledger;
        _memberNotifier = memberNotifier;
        _memberships = memberships;
        _userRepository = userRepository;
        _provider = provider;
        _options = options;
    }

    public virtual async Task<AiStatusDto> GetStatusAsync()
    {
        var caller = await _callerContextFactory.CreateAsync();
        var reason = await _accessGuard.FindWorkspaceBlockAsync(caller) ?? _accessGuard.FindRoleBlock(caller);

        var now = Clock.Now;
        var limits = _options.Value.Quotas.For(caller.PlanName);
        AiQuotaDto? quota = null;
        if (caller.IsOwner || caller.IsManager)
        {
            var month = await _ledger.GetTotalsAsync(caller.WorkspaceId, AiUsageRecordQueries.MonthStart(now));
            quota = new AiQuotaDto
            {
                RequestsUsed = month.Requests,
                RequestsLimit = limits.MonthlyRequests,
                TokensUsed = month.Tokens,
                TokensLimit = limits.MonthlyTokens,
                ResetsAt = NextMonthStart(now)
            };
        }

        if (reason == null && await IsQuotaExhaustedAsync(caller, limits, now))
        {
            reason = AiStatusReasons.QuotaExhausted;
        }

        var features = AiFeatureIds.All
            .Select(feature =>
            {
                var access = _accessGuard.Evaluate(caller, feature);
                return new AiFeatureStatusDto
                {
                    FeatureId = feature.Id,
                    Allowed = reason == null && access.Allowed,
                    Reason = access.Reason,
                    Scopes = access.Allowed ? access.Scopes.ToList() : new List<string>()
                };
            })
            .ToList();

        return new AiStatusDto
        {
            Available = reason == null,
            Reason = reason,
            WorkspaceEnabled = caller.Policy.IsEnabled,
            CanManage = caller.IsOwner,
            ProviderDisplayName = _provider.State == AiProviderState.Configured ? ProviderDisplayName : null,
            DisclosureVersion = caller.DisclosureVersion,
            AllowWindowTitles = caller.Policy.AllowWindowTitles,
            AllowNotes = caller.Policy.AllowNotes,
            Features = features,
            Quota = quota
        };
    }

    public virtual async Task<AiWorkspaceSettingsDto> GetSettingsAsync()
    {
        var workspaceId = CurrentWorkspace.GetId();
        var member = await _memberships.FindActiveAsync(workspaceId, CurrentUser.GetId());
        if (member == null || (member.Role != WorkspaceRoles.Owner && member.Role != WorkspaceRoles.Manager))
        {
            throw new AbpAuthorizationException("Only workspace owners and managers can see the AI assist settings.");
        }

        return await ToSettingsDtoAsync(await _aiSettings.GetPolicyAsync(workspaceId));
    }

    public virtual async Task<AiWorkspaceSettingsDto> UpdateSettingsAsync(UpdateAiWorkspaceSettingsDto input)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var userId = CurrentUser.GetId();
        var member = await _memberships.FindActiveAsync(workspaceId, userId);
        if (member is not { IsOwner: true })
        {
            throw new AbpAuthorizationException("Only the workspace owner can change the AI assist settings.");
        }

        var change = await _aiSettings.UpdateAsync(
            workspaceId,
            userId,
            new AiWorkspaceSettingsUpdate(
                input.IsEnabled, input.AllowManagers, input.AllowMemberSelfService, input.AllowWindowTitles, input.AllowNotes),
            input.AcknowledgedDisclosureVersion);

        if (change.NotifyMembers)
        {
            await _memberNotifier.NotifyAsync(BuildMemberNotification(change));
        }

        return await ToSettingsDtoAsync(change.Policy);
    }

    public virtual async Task<AiUsageSummaryDto> GetUsageAsync()
    {
        var caller = await _callerContextFactory.CreateAsync();
        var now = Clock.Now;
        var monthStart = AiUsageRecordQueries.MonthStart(now);
        var limits = _options.Value.Quotas.For(caller.PlanName);
        var today = await _ledger.GetTotalsAsync(caller.WorkspaceId, AiUsageRecordQueries.DayStart(now), caller.UserId);

        var summary = new AiUsageSummaryDto
        {
            MonthStart = monthStart,
            ResetsAt = NextMonthStart(now),
            MyRequestsToday = today.Requests,
            MyDailyLimit = limits.UserDailyRequests
        };

        if (caller.IsOwner)
        {
            var month = await _ledger.GetTotalsAsync(caller.WorkspaceId, monthStart);
            summary.RequestsUsed = month.Requests;
            summary.RequestsLimit = limits.MonthlyRequests;
            summary.TokensUsed = month.Tokens;
            summary.TokensLimit = limits.MonthlyTokens;
            summary.EstimatedCostUsd = await _ledger.GetCostAsync(caller.WorkspaceId, monthStart);
            summary.ByFeature = (await _ledger.GetByFeatureAsync(caller.WorkspaceId, monthStart))
                .Select(f => new AiFeatureUsageDto { FeatureId = f.FeatureId, Requests = f.Totals.Requests, Tokens = f.Totals.Tokens })
                .ToList();
        }
        else
        {
            var mine = await _ledger.GetTotalsAsync(caller.WorkspaceId, monthStart, caller.UserId);
            summary.RequestsUsed = mine.Requests;
            summary.TokensUsed = mine.Tokens;
        }

        return summary;
    }

    /// <summary>Who processes the data, as the disclosure names them.</summary>
    protected string ProviderDisplayName
    {
        get
        {
            var configured = _options.Value.Disclosure.ProviderDisplayName;
            return !string.IsNullOrWhiteSpace(configured)
                ? configured.Trim()
                : _provider.Descriptor?.DisplayName ?? AiOptions.ProviderOpenAi;
        }
    }

    /// <summary>The workspace's monthly allowance, or the caller's daily one, is used up.</summary>
    protected virtual async Task<bool> IsQuotaExhaustedAsync(AiCallerContext caller, AiQuotaLimits limits, DateTime now)
    {
        if (limits.UserDailyRequests.HasValue)
        {
            var today = await _ledger.GetTotalsAsync(caller.WorkspaceId, AiUsageRecordQueries.DayStart(now), caller.UserId);
            if (today.Requests >= limits.UserDailyRequests.Value)
            {
                return true;
            }
        }

        if (limits.MonthlyRequests.HasValue || limits.MonthlyTokens.HasValue)
        {
            var month = await _ledger.GetTotalsAsync(caller.WorkspaceId, AiUsageRecordQueries.MonthStart(now));
            return month.Requests >= limits.MonthlyRequests.GetValueOrDefault(int.MaxValue) ||
                   month.Tokens >= limits.MonthlyTokens.GetValueOrDefault(long.MaxValue);
        }

        return false;
    }

    protected virtual async Task<AiWorkspaceSettingsDto> ToSettingsDtoAsync(AiWorkspacePolicy policy)
    {
        string? enabledByName = null;
        if (policy.EnabledBy.HasValue)
        {
            var user = await _userRepository.FindAsync(policy.EnabledBy.Value);
            enabledByName = user == null ? null : !string.IsNullOrWhiteSpace(user.Name) ? user.Name : user.UserName;
        }

        return new AiWorkspaceSettingsDto
        {
            IsEnabled = policy.IsEnabled,
            AllowManagers = policy.AllowManagers,
            AllowMemberSelfService = policy.AllowMemberSelfService,
            AllowWindowTitles = policy.AllowWindowTitles,
            AllowNotes = policy.AllowNotes,
            ConsentVersion = policy.ConsentVersion,
            EnabledAt = policy.EnabledAt,
            EnabledByName = enabledByName,
            Disclosure = BuildDisclosure()
        };
    }

    protected virtual AiDisclosureDto BuildDisclosure()
    {
        var disclosure = _options.Value.Disclosure;
        return new AiDisclosureDto
        {
            Version = _aiSettings.CurrentDisclosureVersion,
            ProviderDisplayName = ProviderDisplayName,
            ProcessingRegion = string.IsNullOrWhiteSpace(disclosure.ProcessingRegion) ? null : disclosure.ProcessingRegion.Trim(),
            RetentionNote = string.IsNullOrWhiteSpace(disclosure.RetentionNote) ? null : disclosure.RetentionNote.Trim(),
            AlwaysSent = Localize(AlwaysSentKeys),
            SentWhenAllowed = Localize(SentWhenAllowedKeys),
            NeverSent = Localize(NeverSentKeys),
            WhoCanUse = Localize(WhoCanUseKeys)
        };
    }

    /// <summary>What members are told: that AI assist is on (and what it may send), or that it may now send more.</summary>
    protected virtual string BuildMemberNotification(AiSettingsChange change)
    {
        var added = change.WindowTitlesAdded && change.NotesAdded ? L[WindowTitlesAndNotesCategoryKey].Value
            : change.WindowTitlesAdded ? L[WindowTitlesCategoryKey].Value
            : change.NotesAdded ? L[NotesCategoryKey].Value
            : null;

        if (change.ConsentAction is AiConsentEvent.EnabledAction or AiConsentEvent.ReconsentedAction)
        {
            var enabled = L[EnabledNotificationKey, ProviderDisplayName].Value;
            return added == null ? enabled : enabled + " " + L[AlsoIncludesNotificationKey, added].Value;
        }

        return L[MoreDataNotificationKey, added ?? string.Empty, ProviderDisplayName].Value;
    }

    private List<string> Localize(IEnumerable<string> keys) => keys.Select(key => L[key].Value).ToList();

    private static DateTime NextMonthStart(DateTime now) => AiUsageRecordQueries.MonthStart(now).AddMonths(1);
}
