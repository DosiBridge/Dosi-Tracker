using System;
using System.Collections.Generic;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Projects;

public class Project : FullAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public string Title { get; set; }
    public string? Description { get; set; } // Optional: not every project needs one
    public string Color { get; set; }
    public int IntervalMinutes { get; set; }
    public bool IsArchived { get; set; }

    // Per-project capture permissions pushed to the desktop agents.
    public bool AllowScreenshot { get; set; }
    public bool AllowWebcam { get; set; }
    public bool AllowKeyboard { get; set; }
    public bool AllowMouse { get; set; }
    public bool AllowActiveWindow { get; set; }
    public bool AllowRunningPrograms { get; set; }

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

        // Privacy-first defaults: everything except the webcam.
        AllowScreenshot = true;
        AllowWebcam = false;
        AllowKeyboard = true;
        AllowMouse = true;
        AllowActiveWindow = true;
        AllowRunningPrograms = true;

        Members = new List<ProjectMember>();
    }

    public void Archive() => IsArchived = true;

    public void Unarchive() => IsArchived = false;
}
