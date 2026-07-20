using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Activities;

public class ActivityDto : CreationAuditedEntityDto<Guid>
{
    public Guid UserId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid ClientActivityId { get; set; }
    public DateTime StartedAt { get; set; }
    public DateTime EndedAt { get; set; }
    public int Productivity { get; set; }
    public int MouseClicks { get; set; }
    public int KeyboardHits { get; set; }
    public string Description { get; set; }
    public string ActiveWindowsJson { get; set; }
    public string RunningProgramsJson { get; set; }
}
