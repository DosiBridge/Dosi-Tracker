using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Security.Claims;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Mvc.Abstractions;
using Microsoft.AspNetCore.Mvc.Controllers;
using Microsoft.AspNetCore.Mvc.Filters;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.Workspaces;

/// <summary>The X-Workspace-Id header: members get their workspace (and role), anyone else 403
/// Tracker:NotWorkspaceMember; workspace endpoints called without one 400 Tracker:WorkspaceRequired.</summary>
public class WorkspaceResolutionTests
{
    private static readonly Guid WorkspaceA = Guid.NewGuid();
    private static readonly Guid Member = Guid.NewGuid();

    private sealed class FakeLookup : WorkspaceRoleLookup
    {
        public FakeLookup() : base(null!)
        {
        }

        public override Task<string?> FindRoleAsync(Guid workspaceId, Guid userId)
        {
            return Task.FromResult(workspaceId == WorkspaceA && userId == Member ? WorkspaceRoles.Manager : null);
        }
    }

    private static readonly CurrentWorkspace Current = new();

    private static IServiceProvider Services()
    {
        return new ServiceCollection()
            .AddSingleton<WorkspaceRoleLookup>(new FakeLookup())
            .AddSingleton<ICurrentWorkspace>(Current)
            .BuildServiceProvider();
    }

    private static DefaultHttpContext Request(string? header, Guid? userId)
    {
        var context = new DefaultHttpContext { RequestServices = Services() };
        context.Response.Body = new MemoryStream();
        if (header != null)
        {
            context.Request.Headers[WorkspaceConsts.HeaderName] = header;
        }

        if (userId != null)
        {
            context.User = new ClaimsPrincipal(new ClaimsIdentity(
                new List<Claim> { new(AbpClaimTypes.UserId, userId.Value.ToString()) }, "Bearer"));
        }

        return context;
    }

    private static async Task<(bool NextCalled, Guid? Workspace, string? Role)> RunAsync(HttpContext context)
    {
        var called = false;
        Guid? workspace = null;
        string? role = null;
        var middleware = new WorkspaceResolutionMiddleware(_ =>
        {
            called = true;
            workspace = Current.Id;
            role = Current.Role;
            return Task.CompletedTask;
        });

        await middleware.InvokeAsync(context);
        return (called, workspace, role);
    }

    private static async Task<string> ErrorCodeAsync(HttpContext context)
    {
        context.Response.Body.Position = 0;
        using var json = await JsonDocument.ParseAsync(context.Response.Body);
        return json.RootElement.GetProperty("error").GetProperty("code").GetString()!;
    }

    [Fact]
    public async Task A_Member_Should_Work_In_The_Named_Workspace_With_Their_Role()
    {
        var (called, workspace, role) = await RunAsync(Request(WorkspaceA.ToString(), Member));

        called.ShouldBeTrue();
        workspace.ShouldBe(WorkspaceA);
        role.ShouldBe(WorkspaceRoles.Manager);
        Current.Id.ShouldBeNull(); // only for the request
    }

    [Fact]
    public async Task A_Non_Member_Should_Be_Refused_With_403()
    {
        var context = Request(Guid.NewGuid().ToString(), Member);

        var (called, _, _) = await RunAsync(context);

        called.ShouldBeFalse();
        context.Response.StatusCode.ShouldBe(StatusCodes.Status403Forbidden);
        (await ErrorCodeAsync(context)).ShouldBe(TrackerDomainErrorCodes.NotWorkspaceMember);
        context.Response.Headers["_AbpErrorFormat"].ToString().ShouldBe("true");
    }

    [Fact]
    public async Task A_Malformed_Header_Should_Be_Refused_With_400()
    {
        var context = Request("not-a-guid", Member);

        (await RunAsync(context)).NextCalled.ShouldBeFalse();
        context.Response.StatusCode.ShouldBe(StatusCodes.Status400BadRequest);
        (await ErrorCodeAsync(context)).ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Without_A_Header_Or_A_User_The_Request_Should_Have_No_Workspace()
    {
        (await RunAsync(Request(null, Member))).ShouldBe((true, (Guid?)null, (string?)null));
        (await RunAsync(Request(WorkspaceA.ToString(), null))).ShouldBe((true, (Guid?)null, (string?)null));
    }

    // ---- WorkspaceRequiredFilter ----

    private static readonly WorkspaceRequiredFilter Filter = new(
        typeof(TeamAppService).Assembly,
        typeof(WorkspacesController).Assembly);

    private static ControllerActionDescriptor Action(Type controller, string method)
    {
        return new ControllerActionDescriptor
        {
            ControllerTypeInfo = controller.GetTypeInfo(),
            MethodInfo = controller.GetMethod(method)!
        };
    }

    [Theory]
    [InlineData(typeof(TeamAppService), nameof(TeamAppService.GetMembersAsync), true)]
    [InlineData(typeof(ActivityAppService), nameof(ActivityAppService.CreateAsync), true)]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetSettingsAsync), true)]
    [InlineData(typeof(ScreenshotContentController), nameof(ScreenshotContentController.GetContentAsync), true)]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.RegisterAsync), false)]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetAvailablePlansAsync), false)]
    [InlineData(typeof(WorkspaceAppService), nameof(WorkspaceAppService.GetMyPreferencesAsync), false)]
    [InlineData(typeof(WorkspacesController), nameof(WorkspacesController.GetMineAsync), false)]
    [InlineData(typeof(InvitationsController), nameof(InvitationsController.AcceptAsync), false)]
    [InlineData(typeof(Dosi.Tracker.Platform.PlatformAppService), nameof(Dosi.Tracker.Platform.PlatformAppService.GetOverviewAsync), false)]
    [InlineData(typeof(Volo.Abp.Account.ProfileController), nameof(Volo.Abp.Account.ProfileController.GetAsync), false)]
    public void Endpoints_Should_Require_A_Workspace_Unless_Marked(Type controller, string method, bool required)
    {
        Filter.RequiresWorkspace(Action(controller, method)).ShouldBe(required);
    }

    [Fact]
    public async Task A_Workspace_Endpoint_Without_A_Workspace_Should_Be_Refused()
    {
        var http = new DefaultHttpContext { RequestServices = Services() };
        var context = new ActionExecutingContext(
            new ActionContext(http, new RouteData(), Action(typeof(TeamAppService), nameof(TeamAppService.GetMembersAsync))),
            new List<IFilterMetadata>(),
            new Dictionary<string, object?>(),
            controller: null!);

        var exception = await Should.ThrowAsync<BusinessException>(() =>
            Filter.OnActionExecutionAsync(context, () => Task.FromResult<ActionExecutedContext>(null!)));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);

        using (Current.Change(WorkspaceA))
        {
            var called = false;
            await Filter.OnActionExecutionAsync(context, () =>
            {
                called = true;
                return Task.FromResult<ActionExecutedContext>(null!);
            });
            called.ShouldBeTrue();
        }
    }
}
