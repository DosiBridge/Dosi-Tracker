using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.SaaS;

public class Plan : FullAuditedAggregateRoot<Guid>
{
    public string Name { get; set; }
    public decimal PricePerUser { get; set; }
    public int MaxSeats { get; set; }
    public int TrialDays { get; set; }
    
    protected Plan() { }
    
    public Plan(Guid id, string name, decimal pricePerUser, int maxSeats, int trialDays) 
        : base(id)
    {
        Name = name;
        PricePerUser = pricePerUser;
        MaxSeats = maxSeats;
        TrialDays = trialDays;
    }
}
