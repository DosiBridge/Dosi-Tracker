using System;
using System.Globalization;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Context;
using Microsoft.Extensions.Caching.Distributed;
using Volo.Abp.Caching;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// Generated AI answers by what they were made from (spec 15). The key is
/// <c>ai:{featureId}@{n}:{workspace:N}:{enabledAtTicks}:{audience}:{sha256(scope)}:en:{model}:{sha256(bundle)}</c>:
/// <list type="bullet">
/// <item>the bundle hash makes new data a new key, so entries never go stale, they are just no longer asked for;</item>
/// <item>the time AI assist was turned on is in it, so turning it off and on again drops every entry;</item>
/// <item>the scope (subject, project, range, zone, report, day basis, and the window-title, note and consent settings)
/// is hashed in, so a changed setting is a new key even when the data looks the same;</item>
/// <item>the workspace is in it, so entries are never shared across workspaces. Two readers who see the same bundle may
/// share one, which is safe: each answer is rendered with its reader's own bundle.</item>
/// </list>
/// ABP's distributed cache is in memory per instance unless Redis is configured.
/// </summary>
public class AiResultCache : ITransientDependency
{
    public const string Language = "en";

    private readonly IDistributedCache<AiCachedResultItem, string> _cache;

    public AiResultCache(IDistributedCache<AiCachedResultItem, string> cache)
    {
        _cache = cache;
    }

    /// <param name="promptVersion">"{featureId}@{n}".</param>
    /// <param name="workspaceId">The workspace the answer belongs to.</param>
    /// <param name="policy">The workspace's AI settings (when they were turned on, what they allow).</param>
    /// <param name="scope">What the request is about.</param>
    /// <param name="timeZone">The zone days were counted in.</param>
    /// <param name="model">The model that is asked.</param>
    /// <param name="bundle">What the model is told.</param>
    public static string BuildKey(
        string promptVersion,
        Guid workspaceId,
        AiWorkspacePolicy policy,
        AiAudienceScope scope,
        string timeZone,
        string model,
        AiContextBundle bundle)
    {
        var scopeText = string.Join("|",
            workspaceId.ToString("N"),
            scope.SubjectUserId?.ToString("N") ?? string.Empty,
            scope.ProjectId?.ToString("N") ?? string.Empty,
            Instant(scope.RangeFrom),
            Instant(scope.RangeTo),
            timeZone,
            scope.ReportSlug ?? string.Empty,
            scope.DayBasis ?? string.Empty,
            policy.Flags,
            policy.ConsentVersion ?? string.Empty);

        var enabledAtTicks = (policy.EnabledAt?.Ticks ?? 0L).ToString(CultureInfo.InvariantCulture);

        return string.Join(":",
            "ai",
            promptVersion,
            workspaceId.ToString("N"),
            enabledAtTicks,
            scope.Audience,
            AiCanonicalJson.Sha256Hex(scopeText),
            Language,
            model,
            AiCanonicalJson.Hash(bundle));
    }

    public virtual Task<AiCachedResultItem?> GetAsync(string key, CancellationToken cancellationToken = default)
    {
        return _cache.GetAsync(key, token: cancellationToken);
    }

    /// <summary>Keeps <paramref name="item"/> for <paramref name="ttl"/>; a zero or negative TTL keeps nothing.</summary>
    public virtual async Task SetAsync(string key, AiCachedResultItem item, TimeSpan ttl)
    {
        if (ttl <= TimeSpan.Zero)
        {
            return;
        }

        await _cache.SetAsync(key, item, new DistributedCacheEntryOptions { AbsoluteExpirationRelativeToNow = ttl });
    }

    private static string Instant(DateTime? value)
    {
        if (!value.HasValue)
        {
            return string.Empty;
        }

        var utc = value.Value.Kind == DateTimeKind.Local
            ? value.Value.ToUniversalTime()
            : DateTime.SpecifyKind(value.Value, DateTimeKind.Utc);
        return utc.ToString("O", CultureInfo.InvariantCulture);
    }
}
