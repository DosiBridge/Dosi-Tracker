using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Projects;

public class ProjectMember : CreationAuditedEntity<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid UserId { get; set; }
    public string Role { get; set; } // Worker, Admin, etc.

    /// <summary>Hourly billing rate (workspace currency) used by payroll reporting. 0 = unset.</summary>
    public decimal HourlyRate { get; set; }

    protected ProjectMember()
    {
    }

    public ProjectMember(Guid id, Guid? tenantId, Guid projectId, Guid userId, string role = "Worker", decimal hourlyRate = 0m)
    {
        Id = id;
        TenantId = tenantId;
        ProjectId = projectId;
        UserId = userId;
        Role = role;
        HourlyRate = hourlyRate;
    }
}
