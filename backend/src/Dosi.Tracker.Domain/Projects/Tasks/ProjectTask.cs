using System;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Projects;

/// <summary>
/// A workspace-scoped and project-scoped task or work-item.
/// Supports title, optional description, and completion state lifecycle.
/// </summary>
public class ProjectTask : FullAuditedAggregateRoot<Guid>, IWorkspaceScoped
{
    public Guid WorkspaceId { get; set; }
    public Guid ProjectId { get; set; }
    public string Title { get; set; } = string.Empty;
    public string? Description { get; set; }
    public bool IsCompleted { get; set; }
    public DateTime? CompletedAt { get; set; }

    protected ProjectTask()
    {
    }

    public ProjectTask(
        Guid id,
        Guid workspaceId,
        Guid projectId,
        string title,
        string? description = null,
        bool isCompleted = false,
        DateTime? completedAt = null)
        : base(id)
    {
        WorkspaceId = workspaceId;
        ProjectId = projectId;
        Title = title;
        Description = description;
        IsCompleted = isCompleted;
        CompletedAt = completedAt;
    }

    public void SetTitle(string title)
    {
        Title = title;
    }

    public void SetDescription(string? description)
    {
        Description = description;
    }

    public void Complete(DateTime completedAt)
    {
        IsCompleted = true;
        CompletedAt = completedAt;
    }

    public void Reopen()
    {
        IsCompleted = false;
        CompletedAt = null;
    }
}
