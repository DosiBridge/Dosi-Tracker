using System;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Features;
using Microsoft.Extensions.Caching.Distributed;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Caching;
using Volo.Abp.Modularity;
using Volo.Abp.Timing;
using Xunit;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Ask AI's conversation memory (spec 15): a consent change, or a different user or workspace, ends it with
/// <c>Tracker:AiConversationExpired</c>; it never outlives its absolute cap; and nothing it stores is a real name.
/// </summary>
public abstract class AiConversationStoreTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task A_Second_Turn_Should_Reuse_The_First_Turns_Conversation()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueText("You tracked {F1} this week.");
        var first = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueText("You tracked {F1} this week.");
        var second = await Ai.AskAsync(new AiAskInput { ConversationId = first.ConversationId, Message = "And again?" });

        second.ConversationId.ShouldBe(first.ConversationId);
    }

    [Fact]
    public async Task A_Consent_Change_Should_Expire_The_Conversation()
    {
        await ReadyAsync(titles: false, notes: false);
        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueText("You tracked {F1} this week.");
        var first = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        await SetPolicyAsync(titles: true, notes: false);

        var error = await Should.ThrowAsync<BusinessException>(
            () => Ai.AskAsync(new AiAskInput { ConversationId = first.ConversationId, Message = "And again?" }));
        error.Code.ShouldBe(TrackerDomainErrorCodes.AiConversationExpired);
    }

    [Fact]
    public async Task A_Different_Users_Conversation_Id_Should_Be_Treated_As_Expired()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueText("You tracked {F1} this week.");
        var first = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        await AsUserAsync(AyeshaId, async () =>
        {
            var error = await Should.ThrowAsync<BusinessException>(
                () => Ai.AskAsync(new AiAskInput { ConversationId = first.ConversationId, Message = "Whose conversation is this?" }));
            error.Code.ShouldBe(TrackerDomainErrorCodes.AiConversationExpired);
        });
    }

    [Fact]
    public async Task An_Unknown_Conversation_Id_Should_Be_Treated_As_Expired()
    {
        await ReadyAsync();

        var error = await Should.ThrowAsync<BusinessException>(
            () => Ai.AskAsync(new AiAskInput { ConversationId = Guid.NewGuid(), Message = "Hello?" }));
        error.Code.ShouldBe(TrackerDomainErrorCodes.AiConversationExpired);
    }

    [Fact]
    public async Task The_Stored_Conversation_Should_Never_Hold_A_Real_Name()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", """{"member":"M1"}""")
            .EnqueueText("{M1} tracked {F1} this week.");

        var answer = await Ai.AskAsync(new AiAskInput { Message = "How much did Ayesha Rahman track?" });

        var caller = await WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync());
        var stored = await WithUnitOfWorkAsync(() =>
            GetRequiredService<AiConversationStore>().FindAsync(WorkspaceId, CallerId, answer.ConversationId));

        stored.ShouldNotBeNull();
        var json = JsonSerializer.Serialize(stored);
        json.ShouldNotContain(AyeshaName);
        json.ShouldNotContain("Ayesha");
        json.ShouldContain("M1");
        answer.Answer.ShouldContain(AyeshaName);
    }

    [Fact]
    public async Task A_Conversation_Past_Its_Absolute_Cap_Should_Be_Treated_As_Expired()
    {
        await ReadyAsync();
        var conversationId = Guid.NewGuid();
        var caller = await WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync());
        var item = new AiConversationCacheItem
        {
            CreatedAtUtc = GetRequiredService<IClock>().Now - AiConversationStore.AbsoluteCap - TimeSpan.FromMinutes(1),
            ConsentStamp = AiConversationStore.ConsentStampOf(caller.Policy)
        };

        // Written directly (bypassing AiConversationStore.SaveAsync's own cap guard) to simulate one that has aged
        // past its cap while an instance kept its sliding window alive.
        await GetRequiredService<IDistributedCache<AiConversationCacheItem, string>>().SetAsync(
            AiConversationStore.KeyFor(WorkspaceId, CallerId, conversationId), item,
            new DistributedCacheEntryOptions { AbsoluteExpirationRelativeToNow = TimeSpan.FromHours(1) });

        var found = await WithUnitOfWorkAsync(() =>
            GetRequiredService<AiConversationStore>().FindAsync(WorkspaceId, CallerId, conversationId));

        found.ShouldBeNull();
    }
}
