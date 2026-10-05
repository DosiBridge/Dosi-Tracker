using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Presence;

/// <summary>Live "who is working now" (shared contract §3).</summary>
public interface IPresenceAppService : IApplicationService
{
    /// <summary><c>POST /api/app/presence/heartbeat</c>: records the caller's agent state on one device.</summary>
    Task<HeartbeatResultDto> HeartbeatAsync(HeartbeatDto input);

    /// <summary><c>GET /api/app/presence/team</c>: every member device of the workspace for callers with
    /// Tracker.Activities.ViewAll; only the caller's own devices otherwise.</summary>
    Task<ListResultDto<PresenceDto>> GetTeamAsync();
}
