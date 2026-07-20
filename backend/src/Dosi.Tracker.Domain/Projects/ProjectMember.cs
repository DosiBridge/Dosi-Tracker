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

    protected ProjectMember()
    {
    }

    public ProjectMember(Guid id, Guid? tenantId, Guid projectId, Guid userId, string role = "Worker")
    {
        Id = id;
        TenantId = tenantId;
        ProjectId = projectId;
        UserId = userId;
        Role = role;
    }
}
