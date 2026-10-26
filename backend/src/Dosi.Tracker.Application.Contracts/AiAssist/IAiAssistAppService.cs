using System.Threading.Tasks;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI assist (<c>/api/app/ai-assist/*</c>), in the workspace named by <c>X-Workspace-Id</c>. Off unless this server has
/// a provider, the platform has not switched it off, and the workspace's owner turned it on and consented to the current
/// disclosure. This part: status, settings and usage.
/// </summary>
public partial interface IAiAssistAppService : IApplicationService
{
    /// <summary>GET api/app/ai-assist/status — every member; never fails for AI reasons.</summary>
    Task<AiStatusDto> GetStatusAsync();

    /// <summary>GET api/app/ai-assist/settings — owners and managers.</summary>
    Task<AiWorkspaceSettingsDto> GetSettingsAsync();

    /// <summary>PUT api/app/ai-assist/settings — owners only.</summary>
    Task<AiWorkspaceSettingsDto> UpdateSettingsAsync(UpdateAiWorkspaceSettingsDto input);

    /// <summary>GET api/app/ai-assist/usage — every member (see <see cref="AiUsageSummaryDto"/>).</summary>
    Task<AiUsageSummaryDto> GetUsageAsync();
}
