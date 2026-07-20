using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Projects;

public class ProjectDto : FullAuditedEntityDto<Guid>
{
    public string Title { get; set; }
    public string Description { get; set; }
    public string Color { get; set; }
    public int IntervalMinutes { get; set; }
    public bool IsArchived { get; set; }
}
