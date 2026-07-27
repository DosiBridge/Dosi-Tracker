using Riok.Mapperly.Abstractions;
using Volo.Abp.Mapperly;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Notifications;

namespace Dosi.Tracker;

[Mapper(RequiredMappingStrategy = RequiredMappingStrategy.Target)]
public partial class TrackerApplicationMappers : MapperBase<Plan, PlanDto>
{
    public override partial PlanDto Map(Plan source);
    public override partial void Map(Plan source, PlanDto destination);
}

[Mapper(RequiredMappingStrategy = RequiredMappingStrategy.Target)]
public partial class SubscriptionMapper : MapperBase<Subscription, SubscriptionDto>
{
    public override partial SubscriptionDto Map(Subscription source);
    public override partial void Map(Subscription source, SubscriptionDto destination);
}

[Mapper(RequiredMappingStrategy = RequiredMappingStrategy.Target)]
public partial class InvoiceMapper : MapperBase<Invoice, InvoiceDto>
{
    public override partial InvoiceDto Map(Invoice source);
    public override partial void Map(Invoice source, InvoiceDto destination);
}

[Mapper(RequiredMappingStrategy = RequiredMappingStrategy.Target)]
public partial class ProjectMemberMapper : MapperBase<ProjectMember, ProjectMemberDto>
{
    public override partial ProjectMemberDto Map(ProjectMember source);
    public override partial void Map(ProjectMember source, ProjectMemberDto destination);
}

[Mapper(RequiredMappingStrategy = RequiredMappingStrategy.Target)]
public partial class NotificationMapper : MapperBase<Notification, NotificationDto>
{
    public override partial NotificationDto Map(Notification source);
    public override partial void Map(Notification source, NotificationDto destination);
}
