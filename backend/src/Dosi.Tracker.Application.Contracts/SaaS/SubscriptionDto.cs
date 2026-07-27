using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.SaaS;

public class SubscriptionDto : FullAuditedEntityDto<Guid>
{
    public Guid PlanId { get; set; }
    public string Status { get; set; }
    public DateTime? TrialEndsAt { get; set; }
}
