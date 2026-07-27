using Dosi.Tracker.Localization;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Localization;

namespace Dosi.Tracker.Permissions;

public class TrackerPermissionDefinitionProvider : PermissionDefinitionProvider
{
    public override void Define(IPermissionDefinitionContext context)
    {
        var group = context.AddGroup(TrackerPermissions.GroupName, L("Permission:Tracker"));

        var projects = group.AddPermission(TrackerPermissions.Projects.Default, L("Permission:Projects"));
        projects.AddChild(TrackerPermissions.Projects.Create, L("Permission:Projects.Create"));
        projects.AddChild(TrackerPermissions.Projects.Edit, L("Permission:Projects.Edit"));
        projects.AddChild(TrackerPermissions.Projects.Delete, L("Permission:Projects.Delete"));
        projects.AddChild(TrackerPermissions.Projects.Archive, L("Permission:Projects.Archive"));

        var activities = group.AddPermission(TrackerPermissions.Activities.Default, L("Permission:Activities"));
        activities.AddChild(TrackerPermissions.Activities.ViewAll, L("Permission:Activities.ViewAll"));

        var team = group.AddPermission(TrackerPermissions.Team.Default, L("Permission:Team"));
        team.AddChild(TrackerPermissions.Team.Manage, L("Permission:Team.Manage"));

        group.AddPermission(TrackerPermissions.Billing.Default, L("Permission:Billing"));
        group.AddPermission(TrackerPermissions.Reporting.Default, L("Permission:Reporting"));
    }

    private static LocalizableString L(string name)
    {
        return LocalizableString.Create<TrackerResource>(name);
    }
}
