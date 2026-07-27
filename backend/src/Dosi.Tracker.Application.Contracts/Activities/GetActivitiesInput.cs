using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Activities;

public class GetActivitiesInput : PagedAndSortedResultRequestDto
{
    public Guid? ProjectId { get; set; }

    /// <summary>Only honored for callers with the Tracker.Activities.ViewAll permission;
    /// everyone else is always scoped to their own activities.</summary>
    public Guid? UserId { get; set; }

    public DateTime? From { get; set; }
    public DateTime? To { get; set; }
}
