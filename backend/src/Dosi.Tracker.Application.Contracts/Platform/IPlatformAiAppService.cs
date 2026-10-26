using System.Threading.Tasks;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Platform;

/// <summary>AI assist in the platform console (global <c>platform-admin</c> role): the provider's state, this month's
/// usage across every workspace, and the runtime switch.</summary>
public interface IPlatformAiAppService : IApplicationService
{
    /// <summary>GET api/app/platform-ai/overview.</summary>
    Task<PlatformAiOverviewDto> GetOverviewAsync();

    /// <summary>PUT api/app/platform-ai/status — turns AI assist on or off for every workspace at once (inert while
    /// <c>AI:Enabled</c> is false).</summary>
    Task<PlatformAiStatusDto> UpdateStatusAsync(PlatformAiStatusDto input);
}
