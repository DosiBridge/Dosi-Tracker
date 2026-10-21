using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Projects;

public class ProjectTaskDto : FullAuditedEntityDto<Guid>
{
    public Guid WorkspaceId { get; set; }
    public Guid ProjectId { get; set; }
    public string Title { get; set; } = string.Empty;
    public string? Description { get; set; }
    public bool IsCompleted { get; set; }
    public DateTime? CompletedAt { get; set; }
}
