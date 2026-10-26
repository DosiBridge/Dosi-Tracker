using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Features;
using Dosi.Tracker.AiAssist.Pipeline;
using Volo.Abp.Uow;
using Volo.Abp.Users;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The work helpers (data-quality review, note suggestions, task drafts) and feedback. The helpers only read and
/// suggest; whatever a member keeps is saved through the normal endpoints, which check their permissions again.
/// </summary>
public partial class AiAssistAppService
{
    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiDataQualityReviewDto> CreateDataQualityReviewAsync(AiDataQualityReviewInput input) =>
        Runner.RunAsync(NewFeature<DataQualityReviewFeature>(), input);

    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiNoteSuggestionsDto> CreateNoteSuggestionsAsync(AiNoteSuggestionsInput input) =>
        Runner.RunAsync(NewFeature<NoteSuggestionsFeature>(), input);

    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiTaskDraftsDto> CreateTaskDraftsAsync(AiTaskDraftsInput input) =>
        Runner.RunAsync(NewFeature<TaskDraftsFeature>(), input);

    /// <summary>Rates one of the caller's own results; anyone else's (or an unknown) id is a 404.</summary>
    public virtual Task CreateFeedbackAsync(AiFeedbackInput input) =>
        LazyServiceProvider.LazyGetRequiredService<AiUsageRecorder>()
            .SetFeedbackAsync(input.ResultId, CurrentUser.GetId(), input.Rating, input.Reason);
}
