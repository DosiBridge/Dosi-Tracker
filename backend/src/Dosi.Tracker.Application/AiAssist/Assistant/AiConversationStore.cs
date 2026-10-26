using System;
using System.Globalization;
using System.Threading.Tasks;
using Microsoft.Extensions.Caching.Distributed;
using Volo.Abp.Caching;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Timing;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Reads and writes one Ask AI conversation (spec 15), keyed <c>ai:conv:{workspace:N}:{user:N}:{conversationId:N}</c>.
/// 60 minutes sliding after each turn, never more than <see cref="AbsoluteCap"/> from when the conversation started,
/// whatever the sliding window would otherwise allow. ABP's distributed cache is in memory per instance unless Redis
/// is configured: without it, a follow-up that lands on another instance simply finds nothing, which
/// <see cref="AiAssistantRunner"/> answers as an expired conversation.
/// </summary>
public class AiConversationStore : ITransientDependency
{
    public static readonly TimeSpan AbsoluteCap = TimeSpan.FromHours(4);

    private readonly IDistributedCache<AiConversationCacheItem, string> _cache;
    private readonly IClock _clock;

    public AiConversationStore(IDistributedCache<AiConversationCacheItem, string> cache, IClock clock)
    {
        _cache = cache;
        _clock = clock;
    }

    public static string KeyFor(Guid workspaceId, Guid userId, Guid conversationId) =>
        $"ai:conv:{workspaceId:N}:{userId:N}:{conversationId:N}";

    /// <summary>The consent stamp a workspace's policy carries now: a change to any of it (on/off, consent, the
    /// window-title or notes switch) ends every conversation under the old one.</summary>
    public static string ConsentStampOf(AiWorkspacePolicy policy)
    {
        var enabledAtTicks = (policy.EnabledAt?.Ticks ?? 0L).ToString(CultureInfo.InvariantCulture);
        return string.Join(":", enabledAtTicks, policy.ConsentVersion ?? string.Empty, policy.Flags);
    }

    /// <summary>The conversation, or null when there is none, it expired, or its absolute cap passed.</summary>
    public virtual async Task<AiConversationCacheItem?> FindAsync(Guid workspaceId, Guid userId, Guid conversationId)
    {
        var item = await _cache.GetAsync(KeyFor(workspaceId, userId, conversationId));
        if (item == null)
        {
            return null;
        }

        return _clock.Now - item.CreatedAtUtc > AbsoluteCap ? null : item;
    }

    /// <summary>Keeps <paramref name="item"/>: 60 minutes (<c>AI:Assistant:ConversationTtlMinutes</c>) sliding, never
    /// past <see cref="AbsoluteCap"/> from <see cref="AiConversationCacheItem.CreatedAtUtc"/>.</summary>
    public virtual async Task SaveAsync(Guid workspaceId, Guid userId, Guid conversationId, AiConversationCacheItem item, int ttlMinutes)
    {
        var remaining = AbsoluteCap - (_clock.Now - item.CreatedAtUtc);
        if (remaining <= TimeSpan.Zero)
        {
            // Past its absolute cap: let it fall out rather than write something already stale.
            return;
        }

        var sliding = TimeSpan.FromMinutes(Math.Max(1, ttlMinutes));
        await _cache.SetAsync(KeyFor(workspaceId, userId, conversationId), item, new DistributedCacheEntryOptions
        {
            SlidingExpiration = sliding,
            AbsoluteExpirationRelativeToNow = remaining < sliding ? remaining : sliding
        });
    }
}
