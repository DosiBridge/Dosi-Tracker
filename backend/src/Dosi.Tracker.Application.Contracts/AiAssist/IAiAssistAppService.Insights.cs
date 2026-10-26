using System.Threading.Tasks;

namespace Dosi.Tracker.AiAssist;

/// <summary>The insight cards. Each is an explicit request (nothing generates on its own), answered by the model, from
/// the cache, by a deterministic summary when the model is unavailable ("fallback"), or "no-data" without any call.</summary>
public partial interface IAiAssistAppService
{
    /// <summary>POST api/app/ai-assist/period-summary — the team or a project (owners and managers who see the team),
    /// or the caller's own time; at most 31 days.</summary>
    Task<AiPeriodSummaryDto> CreatePeriodSummaryAsync(AiPeriodSummaryInput input);

    /// <summary>POST api/app/ai-assist/member-day-summary — one member's local day (another member only for a caller
    /// who sees the team).</summary>
    Task<AiMemberDaySummaryDto> CreateMemberDaySummaryAsync(AiMemberDaySummaryInput input);

    /// <summary>POST api/app/ai-assist/report-explanation — owners and managers who see the team; payroll is never
    /// explained.</summary>
    Task<AiReportExplanationDto> CreateReportExplanationAsync(AiReportExplanationInput input);
}
