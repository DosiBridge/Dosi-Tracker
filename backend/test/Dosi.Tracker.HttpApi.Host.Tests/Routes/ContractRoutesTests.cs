using System;
using System.Linq;
using System.Reflection;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Platform;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.SaaS;
using Microsoft.AspNetCore.Authorization;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Routes;

/// <summary>
/// The HTTP routes of the shared contract (agents, API and web), as ABP derives them from the application services.
/// Ids other than <c>id</c> travel in the query string (<c>?tenantId=</c>, <c>?invoiceId=</c>): a route segment
/// instead would break every client built from the contract. The team routes are covered by
/// <see cref="Teams.TeamRoutesTests"/>.
/// </summary>
public class ContractRoutesTests
{
    [Theory]
    // §1 upload and §5 activities read API
    [InlineData(typeof(ActivityAppService), nameof(ActivityAppService.CreateAsync), "POST", "api/app/activity")]
    [InlineData(typeof(ActivityAppService), nameof(ActivityAppService.GetListAsync), "GET", "api/app/activity")]
    [InlineData(typeof(ActivityAppService), nameof(ActivityAppService.UpdateNoteAsync), "PUT", "api/app/activity/{id}/note")]
    // §2 projects for agents
    [InlineData(typeof(ProjectAppService), nameof(ProjectAppService.GetMyProjectsAsync), "GET", "api/app/project/my-projects")]
    // §3 presence
    [InlineData(typeof(PresenceAppService), nameof(PresenceAppService.HeartbeatAsync), "POST", "api/app/presence/heartbeat")]
    [InlineData(typeof(PresenceAppService), nameof(PresenceAppService.GetTeamAsync), "GET", "api/app/presence/team")]
    // §6 reporting
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetSummaryAsync), "GET", "api/app/reporting/summary")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetDailySeriesAsync), "GET", "api/app/reporting/daily-series")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetUserDailySeriesAsync), "GET", "api/app/reporting/user-daily-series")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetAppUsageAsync), "GET", "api/app/reporting/app-usage")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetAttendanceAsync), "GET", "api/app/reporting/attendance")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetPayrollAsync), "GET", "api/app/reporting/payroll")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetHourlyAsync), "GET", "api/app/reporting/hourly")]
    [InlineData(typeof(ReportingAppService), nameof(ReportingAppService.GetDevicesAsync), "GET", "api/app/reporting/devices")]
    // §7 workspace settings, preferences and sign-up
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetSettingsAsync), "GET", "api/app/workspace/settings")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.UpdateSettingsAsync), "PUT", "api/app/workspace/settings")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetMyPreferencesAsync), "GET", "api/app/workspace/my-preferences")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.UpdateMyPreferencesAsync), "PUT", "api/app/workspace/my-preferences")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetAvailablePlansAsync), "GET", "api/app/workspace/available-plans")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.RegisterAsync), "POST", "api/app/workspace/register")]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetCurrentSubscriptionAsync), "GET", "api/app/workspace/current-subscription")]
    // §7 host console
    [InlineData(typeof(PlatformAppService), nameof(PlatformAppService.ChangeWorkspacePlanAsync), "POST", "api/app/platform/change-workspace-plan")]
    [InlineData(typeof(PlatformAppService), nameof(PlatformAppService.ActivateSubscriptionAsync), "POST", "api/app/platform/activate-subscription")]
    [InlineData(typeof(PlatformAppService), nameof(PlatformAppService.SuspendSubscriptionAsync), "POST", "api/app/platform/suspend-subscription")]
    [InlineData(typeof(PlatformAppService), nameof(PlatformAppService.MarkInvoicePaidAsync), "POST", "api/app/platform/mark-invoice-paid")]
    [InlineData(typeof(PlatformAppService), nameof(PlatformAppService.GetWorkspaceSubscriptionsAsync), "GET", "api/app/platform/workspace-subscriptions")]
    public void Contract_Endpoints_Should_Have_The_Contract_Routes(Type appService, string methodName, string verb, string route)
    {
        ConventionalRoutes.Of(appService, methodName).ShouldBe((verb, route));
    }

    [Theory]
    [InlineData(nameof(WorkspaceAppService.GetMineAsync))]
    [InlineData(nameof(WorkspaceAppService.CreateAsync))]
    [InlineData(nameof(WorkspaceAppService.LeaveAsync))]
    [InlineData(nameof(WorkspaceAppService.GetInvitationAsync))]
    [InlineData(nameof(WorkspaceAppService.AcceptInvitationAsync))]
    [InlineData(nameof(WorkspaceAppService.DeclineInvitationAsync))]
    public void Workspace_And_Invitation_Endpoints_Should_Be_Served_By_Controllers_Only(string methodName)
    {
        typeof(WorkspaceAppService).GetMethod(methodName)!
            .GetCustomAttribute<Volo.Abp.RemoteServiceAttribute>()!.IsEnabled.ShouldBeFalse();
    }

    [Theory]
    [InlineData(typeof(Dosi.Tracker.Controllers.WorkspacesController), "GetMineAsync", "GET", "mine")]
    [InlineData(typeof(Dosi.Tracker.Controllers.WorkspacesController), "CreateAsync", "POST", "")]
    [InlineData(typeof(Dosi.Tracker.Controllers.WorkspacesController), "LeaveAsync", "POST", "{id}/leave")]
    [InlineData(typeof(Dosi.Tracker.Controllers.InvitationsController), "GetAsync", "GET", "{token}")]
    [InlineData(typeof(Dosi.Tracker.Controllers.InvitationsController), "AcceptAsync", "POST", "{token}/accept")]
    [InlineData(typeof(Dosi.Tracker.Controllers.InvitationsController), "DeclineAsync", "POST", "{token}/decline")]
    public void Workspace_And_Invitation_Controllers_Should_Have_The_Contract_Routes(Type controller, string methodName, string verb, string template)
    {
        var prefix = controller == typeof(Dosi.Tracker.Controllers.WorkspacesController) ? "api/app/workspaces" : "api/app/invitations";
        controller.GetCustomAttribute<Microsoft.AspNetCore.Mvc.RouteAttribute>()!.Template.ShouldBe(prefix);
        controller.GetCustomAttribute<Dosi.Tracker.Workspaces.WorkspaceNotRequiredAttribute>().ShouldNotBeNull();

        var method = controller.GetMethod(methodName)!;
        var http = method.GetCustomAttributes().OfType<Microsoft.AspNetCore.Mvc.Routing.HttpMethodAttribute>().Single();
        http.HttpMethods.ShouldBe(new[] { verb });
        (http.Template ?? string.Empty).ShouldBe(template);
    }

    [Fact]
    public void The_Workspace_Switch_Grant_Should_Be_Gone()
    {
        Dosi.Tracker.OpenIddict.OpenIddictDataSeedContributor.AppClientGrantTypes().ShouldNotContain("urn:dosi:workspace_switch");
    }

    [Fact]
    public void The_Sign_Up_Page_Should_Read_The_Plans_Anonymously()
    {
        typeof(WorkspaceAppService).GetMethod(nameof(WorkspaceAppService.GetAvailablePlansAsync))!
            .GetCustomAttribute<AllowAnonymousAttribute>().ShouldNotBeNull();
    }
}
