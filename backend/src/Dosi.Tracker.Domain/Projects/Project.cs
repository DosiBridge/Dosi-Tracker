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

    /// <summary>When the project was archived (null while active). Blocks that started before it are still accepted.</summary>
    public DateTime? ArchivedAt { get; protected set; }

    // Per-project capture permissions pushed to the desktop agents.
    public bool AllowScreenshot { get; set; }
    public bool AllowWebcam { get; set; }
    public bool AllowKeyboard { get; set; }
    public bool AllowMouse { get; set; }
    public bool AllowActiveWindow { get; set; }
    public bool AllowRunningPrograms { get; set; }

    // Idle and capture policy pushed to the desktop agents.
    /// <summary>Minutes without input after which the agents end the block as idle; 0 turns idle detection off.</summary>
    public int IdleThresholdMinutes { get; set; }

    /// <summary>The agents must blur screen captures (and thumbnails) before uploading them.</summary>
    public bool BlurScreenshots { get; set; }

    public ICollection<ProjectMember> Members { get; set; }

    protected Project()
    {
    }

    public Project(Guid id, Guid? tenantId, string title, int intervalMinutes = 10, string color = ProjectConsts.DefaultColor)
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

        IdleThresholdMinutes = 5;
        BlurScreenshots = false;

        Members = new List<ProjectMember>();
    }

    /// <summary>Archives the project as of <paramref name="archivedAt"/> (the caller's clock). Archiving an
    /// already archived project keeps the original moment, so a repeated call never moves the cut-off.</summary>
    public void Archive(DateTime archivedAt)
    {
        if (IsArchived && ArchivedAt.HasValue)
        {
            return;
        }

        IsArchived = true;
        ArchivedAt = archivedAt;
    }

    public void Unarchive()
    {
        IsArchived = false;
        ArchivedAt = null;
    }
}
