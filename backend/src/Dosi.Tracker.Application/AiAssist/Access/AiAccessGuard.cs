using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.SaaS;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Access;

/// <summary>Whether a caller may use one feature (<see cref="AiAccessGuard.Evaluate"/>).</summary>
/// <param name="Reason">One of <see cref="AiFeatureReasons"/> when not allowed.</param>
/// <param name="Scopes">What the caller may ask about (<see cref="AiFeatureScopes"/>); empty for features without scopes.</param>
public sealed record AiFeatureAccess(bool Allowed, string? Reason, IReadOnlyList<string> Scopes)
{
    public static AiFeatureAccess Refused(string reason) => new(false, reason, Array.Empty<string>());
}

/// <summary>
/// Decides whether a caller may use AI assist, before anything is read or sent. <see cref="EnsureAsync"/> refuses in a
/// fixed order: no provider (<c>Tracker:AiNotConfigured</c>), the feature switched off on this server
/// (<c>Tracker:AiFeatureDisabled</c> operator), the platform switch (<c>Tracker:AiPlatformDisabled</c>), the subscription
/// (<c>Tracker:SubscriptionInactive</c>), the workspace off or its consent stale (<c>Tracker:AiDisabled</c> off /
/// reconsent), then the caller's role (<c>Tracker:AiFeatureDisabled</c> role / self-service-off). The status endpoint
/// reports the same checks without throwing.
/// </summary>
public class AiAccessGuard : ITransientDependency
{
    private static readonly string[] TeamAndSelf = { AiFeatureScopes.Team, AiFeatureScopes.Self };
    private static readonly string[] MemberAndSelf = { AiFeatureScopes.Member, AiFeatureScopes.Self };
    private static readonly string[] SelfOnly = { AiFeatureScopes.Self };

    private readonly IAiChatClientProvider _provider;
    private readonly IOptions<AiOptions> _options;
    private readonly AiSettingsManager _settings;
    private readonly ISubscriptionAccessChecker _subscriptionAccessChecker;

    public AiAccessGuard(
        IAiChatClientProvider provider,
        IOptions<AiOptions> options,
        AiSettingsManager settings,
        ISubscriptionAccessChecker subscriptionAccessChecker)
    {
        _provider = provider;
        _options = options;
        _settings = settings;
        _subscriptionAccessChecker = subscriptionAccessChecker;
    }

    /// <summary>
    /// The first check that keeps AI assist from the whole workspace (<see cref="AiStatusReasons"/>: not-configured,
    /// platform-disabled, subscription-inactive, workspace-disabled, reconsent-required), or null when none does.
    /// </summary>
    public virtual async Task<string?> FindWorkspaceBlockAsync(AiCallerContext caller)
    {
        if (_provider.State != AiProviderState.Configured)
        {
            return AiStatusReasons.NotConfigured;
        }

        if (!await _settings.IsPlatformEnabledAsync())
        {
            return AiStatusReasons.PlatformDisabled;
        }

        if (!await _subscriptionAccessChecker.IsUsableAsync())
        {
            return AiStatusReasons.SubscriptionInactive;
        }

        if (!caller.Policy.IsEnabled)
        {
            return AiStatusReasons.WorkspaceDisabled;
        }

        return caller.Policy.NeedsReconsent(caller.DisclosureVersion) ? AiStatusReasons.ReconsentRequired : null;
    }

    /// <summary>Why the caller's role keeps every feature from them (<see cref="AiStatusReasons.RoleNotAllowed"/> or
    /// <see cref="AiStatusReasons.SelfServiceOff"/>), or null.</summary>
    public virtual string? FindRoleBlock(AiCallerContext caller)
    {
        if (caller.Role == null || caller.IsClientOnly || (caller.IsManager && !caller.Policy.AllowManagers))
        {
            return AiStatusReasons.RoleNotAllowed;
        }

        return caller.IsMember && !caller.Policy.AllowMemberSelfService ? AiStatusReasons.SelfServiceOff : null;
    }

    /// <summary>
    /// What one feature allows the caller, apart from the workspace-wide checks: whether this server offers it, whether
    /// it is for the caller's role, the permissions it needs, and what it may be asked about. Never throws for a known
    /// feature.
    /// </summary>
    public virtual AiFeatureAccess Evaluate(AiCallerContext caller, AiFeature feature)
    {
        Check.NotNull(caller, nameof(caller));
        Check.NotNull(feature, nameof(feature));

        if (!IsOfferedByOperator(feature))
        {
            return AiFeatureAccess.Refused(AiFeatureReasons.Operator);
        }

        switch (FindRoleBlock(caller))
        {
            case AiStatusReasons.RoleNotAllowed:
                return AiFeatureAccess.Refused(AiFeatureReasons.Role);
            case AiStatusReasons.SelfServiceOff:
                return AiFeatureAccess.Refused(AiFeatureReasons.SelfServiceOff);
        }

        switch (feature.Id)
        {
            case AiFeatureIds.PeriodSummaryId:
            case AiFeatureIds.DataQualityReviewId:
                return new AiFeatureAccess(true, null, caller.CanSeeTeam ? TeamAndSelf : SelfOnly);
            case AiFeatureIds.MemberDaySummaryId:
                return new AiFeatureAccess(true, null, caller.CanSeeTeam ? MemberAndSelf : SelfOnly);
            case AiFeatureIds.ReportExplanationId:
                if (caller.IsMember)
                {
                    return AiFeatureAccess.Refused(AiFeatureReasons.Role);
                }

                return caller.CanReport ? Allowed() : AiFeatureAccess.Refused(AiFeatureReasons.Permission);
            case AiFeatureIds.TaskDraftsId:
                var mayDraft = caller.IsMember ? caller.HasWorkerProjectMembership : caller.CanEditProjects;
                return mayDraft ? Allowed() : AiFeatureAccess.Refused(AiFeatureReasons.Permission);
            default:
                return Allowed();
        }
    }

    /// <summary>Throws unless the caller may use <paramref name="featureId"/> now (the order is in the class remarks).
    /// Every refusal carries <c>data.featureId</c>, and the reason where the code has one.</summary>
    public virtual async Task EnsureAsync(AiCallerContext caller, string featureId)
    {
        Check.NotNull(caller, nameof(caller));
        var feature = AiFeatureIds.Find(featureId)
                      ?? throw new ArgumentException($"'{featureId}' is not an AI assist feature.", nameof(featureId));

        if (_provider.State != AiProviderState.Configured)
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiNotConfigured).WithData("featureId", feature.Id);
        }

        if (!IsOfferedByOperator(feature))
        {
            throw FeatureDisabled(feature, AiFeatureReasons.Operator);
        }

        if (!await _settings.IsPlatformEnabledAsync())
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiPlatformDisabled).WithData("featureId", feature.Id);
        }

        if (!await _subscriptionAccessChecker.IsUsableAsync())
        {
            throw new BusinessException(TrackerDomainErrorCodes.SubscriptionInactive).WithData("featureId", feature.Id);
        }

        if (!caller.Policy.IsEnabled || caller.Policy.NeedsReconsent(caller.DisclosureVersion))
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiDisabled)
                .WithData("reason", caller.Policy.IsEnabled ? "reconsent" : "off")
                .WithData("featureId", feature.Id);
        }

        var access = Evaluate(caller, feature);
        if (access.Allowed)
        {
            return;
        }

        if (access.Reason == AiFeatureReasons.Permission)
        {
            throw new AbpAuthorizationException($"The {feature.Id} AI feature needs a permission you do not have.");
        }

        throw FeatureDisabled(feature, access.Reason!);
    }

    /// <summary>The feature's own switch (<c>AI:Features:{ConfigKey}:Enabled</c>); Ask AI also needs the assistant on
    /// and a model that can call tools.</summary>
    protected virtual bool IsOfferedByOperator(AiFeature feature)
    {
        var options = _options.Value;
        if (!options.GetFeature(feature).Enabled)
        {
            return false;
        }

        return feature.Id != AiFeatureIds.AskId ||
               (options.Assistant.Enabled && options.SupportsTools && (_provider.Descriptor?.SupportsTools ?? true));
    }

    private static AiFeatureAccess Allowed() => new(true, null, Array.Empty<string>());

    private static BusinessException FeatureDisabled(AiFeature feature, string reason) =>
        new BusinessException(TrackerDomainErrorCodes.AiFeatureDisabled)
            .WithData("reason", reason)
            .WithData("featureId", feature.Id);
}
