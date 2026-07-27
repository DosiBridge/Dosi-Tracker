using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.SaaS;

public class PlanDto : FullAuditedEntityDto<Guid>
{
    public string Name { get; set; }
    public decimal PricePerUser { get; set; }
    public int MaxSeats { get; set; }
    public int TrialDays { get; set; }
}
