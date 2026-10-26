using System.Threading.Tasks;

namespace Dosi.Tracker.AiAssist;

/// <summary>AI help with the work itself: a data-quality review, note suggestions and task drafts (all read-only: what
/// a member keeps is saved through the normal endpoints), and feedback on any AI result.</summary>
public partial interface IAiAssistAppService
{
    /// <summary>POST api/app/ai-assist/data-quality-review — the team (owners and managers who see the team) or the
    /// caller's own data; at most 31 days.</summary>
    Task<AiDataQualityReviewDto> CreateDataQualityReviewAsync(AiDataQualityReviewInput input);

    /// <summary>POST api/app/ai-assist/note-suggestions — for a block whose note the caller may change (and, when it is
    /// someone else's, may read).</summary>
    Task<AiNoteSuggestionsDto> CreateNoteSuggestionsAsync(AiNoteSuggestionsInput input);

    /// <summary>POST api/app/ai-assist/task-drafts — for someone who may add the project's tasks.</summary>
    Task<AiTaskDraftsDto> CreateTaskDraftsAsync(AiTaskDraftsInput input);

    /// <summary>POST api/app/ai-assist/feedback — rates one of the caller's own results (204).</summary>
    Task CreateFeedbackAsync(AiFeedbackInput input);
}
