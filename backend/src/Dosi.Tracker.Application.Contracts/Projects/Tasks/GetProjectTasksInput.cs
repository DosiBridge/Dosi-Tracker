using System;
using System.ComponentModel.DataAnnotations;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Projects;

public class GetProjectTasksInput : PagedResultRequestDto
{
    [Required]
    public Guid ProjectId { get; set; }

    public bool? IsCompleted { get; set; }
}
