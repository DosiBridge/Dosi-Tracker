using System;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>Helpful / not helpful: only the member who received a result may rate it.</summary>
public abstract class FeedbackTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private Task<AiUsageRecord> RowAsync(Guid id) =>
        WithUnitOfWorkAsync(() => GetRequiredService<IRepository<AiUsageRecord, Guid>>().GetAsync(id));

    [Fact]
    public async Task The_Member_Who_Received_A_Result_Should_Rate_It()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        var mine = await InsertUsageAsync(CallerId);

        await GetRequiredService<IAiAssistAppService>().CreateFeedbackAsync(
            new AiFeedbackInput { ResultId = mine.Id, Rating = AiUsageRecord.FeedbackDown, Reason = "inaccurate" });

        var row = await RowAsync(mine.Id);
        row.FeedbackRating.ShouldBe(AiUsageRecord.FeedbackDown);
        row.FeedbackReason.ShouldBe("inaccurate");

        await GetRequiredService<IAiAssistAppService>().CreateFeedbackAsync(new AiFeedbackInput { ResultId = mine.Id, Rating = AiUsageRecord.FeedbackUp });
        (await RowAsync(mine.Id)).FeedbackRating.ShouldBe(AiUsageRecord.FeedbackUp);
        (await RowAsync(mine.Id)).FeedbackReason.ShouldBeNull();
    }

    [Fact]
    public async Task Someone_Elses_Or_An_Unknown_Result_Should_Be_Not_Found()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        var someoneElse = await CreateMemberAsync("ai.other", WorkspaceRoles.Member);
        var theirs = await InsertUsageAsync(someoneElse);

        await Should.ThrowAsync<EntityNotFoundException>(() => GetRequiredService<IAiAssistAppService>()
            .CreateFeedbackAsync(new AiFeedbackInput { ResultId = theirs.Id, Rating = AiUsageRecord.FeedbackUp }));
        await Should.ThrowAsync<EntityNotFoundException>(() => GetRequiredService<IAiAssistAppService>()
            .CreateFeedbackAsync(new AiFeedbackInput { ResultId = Guid.NewGuid(), Rating = AiUsageRecord.FeedbackUp }));

        (await RowAsync(theirs.Id)).FeedbackRating.ShouldBeNull();
    }

    [Fact]
    public void Feedback_Should_Stay_Transactional_While_The_Ai_Calls_Do_Not()
    {
        var service = typeof(AiAssistAppService);
        service.GetMethod(nameof(AiAssistAppService.CreateFeedbackAsync))!.GetCustomAttribute<UnitOfWorkAttribute>().ShouldBeNull();

        foreach (var name in new[]
                 {
                     nameof(AiAssistAppService.CreatePeriodSummaryAsync), nameof(AiAssistAppService.CreateMemberDaySummaryAsync),
                     nameof(AiAssistAppService.CreateReportExplanationAsync), nameof(AiAssistAppService.CreateDataQualityReviewAsync),
                     nameof(AiAssistAppService.CreateNoteSuggestionsAsync), nameof(AiAssistAppService.CreateTaskDraftsAsync)
                 })
        {
            var attribute = service.GetMethod(name)!.GetCustomAttribute<UnitOfWorkAttribute>().ShouldNotBeNull(name);
            attribute.IsTransactional.ShouldBe(false, name);
        }

        service.GetMethods().Where(m => m.Name.StartsWith("Create", StringComparison.Ordinal) && m.Name.EndsWith("Async", StringComparison.Ordinal))
            .Select(m => m.Name).Distinct().Count().ShouldBe(7);
    }
}
