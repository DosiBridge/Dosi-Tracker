using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Projects;

public class CreateUpdateProjectDto
{
    [Required]
    [MaxLength(128)]
    public string Title { get; set; }

    public string Description { get; set; }

    [Required]
    public string Color { get; set; } = "#006bff";

    [Required]
    public int IntervalMinutes { get; set; } = 10;
}
