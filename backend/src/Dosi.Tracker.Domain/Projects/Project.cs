using System;
using System.Collections.Generic;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Projects;

public class Project : FullAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public string Title { get; set; }
    public string Description { get; set; }
    public string Color { get; set; }
    public int IntervalMinutes { get; set; }
    public bool IsArchived { get; set; }

    public ICollection<ProjectMember> Members { get; set; }

    protected Project()
    {
    }

    public Project(Guid id, Guid? tenantId, string title, int intervalMinutes = 10, string color = "#006bff")
        : base(id)
    {
        TenantId = tenantId;
        Title = title;
        IntervalMinutes = intervalMinutes;
        Color = color;
        IsArchived = false;
        Members = new List<ProjectMember>();
    }
}
