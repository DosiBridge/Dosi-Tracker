using System;
using Volo.Abp.Domain.Entities.Auditing;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Projects;

/// <summary>
/// One entry of a member's hourly-rate history on a project. Rates are effective-dated: payroll prices a
/// block with the latest rate whose <see cref="EffectiveFrom"/> is not after the block's start, so changing
/// a rate never re-prices time tracked before the change. Rows are only ever appended.
/// </summary>
public class ProjectMemberRate : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public Guid WorkspaceId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid UserId { get; set; }

    /// <summary>Hourly billing rate (workspace currency); same unit as <see cref="ProjectMember.HourlyRate"/>.</summary>
    public decimal HourlyRate { get; set; }

    /// <summary>UTC moment from which this rate applies.</summary>
    public DateTime EffectiveFrom { get; set; }

    protected ProjectMemberRate()
    {
    }

    public ProjectMemberRate(Guid id, Guid workspaceId, Guid projectId, Guid userId, decimal hourlyRate, DateTime effectiveFrom)
    {
        Id = id;
        WorkspaceId = workspaceId;
        ProjectId = projectId;
        UserId = userId;
        HourlyRate = hourlyRate;
        EffectiveFrom = effectiveFrom;
    }
}
