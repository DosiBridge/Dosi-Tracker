using System;
using System.Linq;
using System.Reflection;
using System.Security.Principal;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc.Controllers;
using Microsoft.AspNetCore.Mvc.Filters;
using Dosi.Tracker.Devices;
using Microsoft.Extensions.DependencyInjection;
using Volo.Abp;

namespace Dosi.Tracker.Workspaces;

/// <summary>
/// Resolves the workspace of a request from the <c>X-Workspace-Id</c> header: a signed-in caller must be an active
/// member of it (otherwise 403 <c>Tracker:NotWorkspaceMember</c>), and the request then runs in it with the caller's
/// role (<see cref="ICurrentWorkspace"/>), which is also what <c>/api/abp/application-configuration</c> reports
/// permissions for. A token bound to a desktop agent install that is revoked in the workspace is refused there
/// (403 <c>Tracker:DeviceRevoked</c>). An anonymous request's header is ignored. Runs after authentication.
/// </summary>
public class WorkspaceResolutionMiddleware
{
    private readonly RequestDelegate _next;

    public WorkspaceResolutionMiddleware(RequestDelegate next)
    {
        _next = next;
    }

    public async Task InvokeAsync(HttpContext context)
    {
        var header = context.Request.Headers[WorkspaceConsts.HeaderName].ToString();
        if (string.IsNullOrWhiteSpace(header))
        {
            await _next(context);
            return;
        }

        if (!Guid.TryParse(header.Trim(), out var workspaceId) || workspaceId == Guid.Empty)
        {
            await WriteErrorAsync(context, StatusCodes.Status400BadRequest, TrackerDomainErrorCodes.WorkspaceRequired,
                $"The {WorkspaceConsts.HeaderName} header is not a valid workspace id.");
            return;
        }

        var userId = context.User.FindUserId();
        if (userId == null)
        {
            await _next(context);
            return;
        }

        var lookup = context.RequestServices.GetRequiredService<WorkspaceRoleLookup>();
        var role = await lookup.FindRoleAsync(workspaceId, userId.Value);
        if (role == null)
        {
            await WriteErrorAsync(context, StatusCodes.Status403Forbidden, TrackerDomainErrorCodes.NotWorkspaceMember,
                "You are not a member of this workspace.");
            return;
        }

        var isArchived = await lookup.IsArchivedAsync(workspaceId);
        if (isArchived)
        {
            var endpoint = context.GetEndpoint();
            var action = endpoint?.Metadata.GetMetadata<ControllerActionDescriptor>();
            var isNotRequired = action != null &&
                (action.ControllerTypeInfo.IsDefined(typeof(WorkspaceNotRequiredAttribute), inherit: true) ||
                 action.MethodInfo.IsDefined(typeof(WorkspaceNotRequiredAttribute), inherit: true));

            if (!isNotRequired)
            {
                await WriteErrorAsync(context, StatusCodes.Status403Forbidden, TrackerDomainErrorCodes.WorkspaceArchived,
                    "This workspace is archived.");
                return;
            }
        }

        using (context.RequestServices.GetRequiredService<ICurrentWorkspace>().Change(workspaceId, role))
        {
            // A desktop agent's token names its install; a revoked install may do nothing more in this workspace (its
            // sessions were revoked with it, but an access token stays valid until it expires).
            var deviceId = DeviceTokenBinding.Parse(context.User.FindFirst(DeviceConsts.TokenClaimType)?.Value);
            if (deviceId != null && await context.RequestServices.GetRequiredService<DeviceRevocationManager>()
                    .IsDeviceRevokedAsync(workspaceId, userId.Value, deviceId.Value))
            {
                await WriteErrorAsync(context, StatusCodes.Status403Forbidden, TrackerDomainErrorCodes.DeviceRevoked,
                    "This computer was removed from this workspace.");
                return;
            }

            await _next(context);
        }
    }

    /// <summary>An error in ABP's remote-service error format.</summary>
    public static async Task WriteErrorAsync(HttpContext context, int statusCode, string code, string message)
    {
        context.Response.StatusCode = statusCode;
        context.Response.ContentType = "application/json";
        context.Response.Headers["_AbpErrorFormat"] = "true";
        await context.Response.WriteAsync(JsonSerializer.Serialize(new { error = new { code, message } }));
    }
}

/// <summary>Answers 400 <c>Tracker:WorkspaceRequired</c> when a tracker endpoint that works inside a workspace is
/// called without one, or 403 <c>Tracker:WorkspaceArchived</c> when the target workspace is archived.
/// Endpoints (or their classes) marked <see cref="WorkspaceNotRequiredAttribute"/> are exempt, and
/// so is everything outside the tracker's own assemblies (ABP's account, identity and configuration endpoints).</summary>
public class WorkspaceRequiredFilter : IAsyncActionFilter
{
    private readonly Assembly[] _trackerAssemblies;

    public WorkspaceRequiredFilter(params Assembly[] trackerAssemblies)
    {
        _trackerAssemblies = trackerAssemblies;
    }

    public async Task OnActionExecutionAsync(ActionExecutingContext context, ActionExecutionDelegate next)
    {
        if (RequiresWorkspace(context.ActionDescriptor as ControllerActionDescriptor))
        {
            var currentWorkspace = context.HttpContext.RequestServices.GetRequiredService<ICurrentWorkspace>();
            if (currentWorkspace.Id == null)
            {
                throw new BusinessException(TrackerDomainErrorCodes.WorkspaceRequired);
            }

            var lookup = context.HttpContext.RequestServices.GetRequiredService<WorkspaceRoleLookup>();
            if (await lookup.IsArchivedAsync(currentWorkspace.Id.Value))
            {
                throw new BusinessException(TrackerDomainErrorCodes.WorkspaceArchived);
            }
        }

        await next();
    }

    public bool RequiresWorkspace(ControllerActionDescriptor? action)
    {
        if (action == null || !_trackerAssemblies.Contains(action.ControllerTypeInfo.Assembly))
        {
            return false;
        }

        return !action.ControllerTypeInfo.IsDefined(typeof(WorkspaceNotRequiredAttribute), inherit: true) &&
               !action.MethodInfo.IsDefined(typeof(WorkspaceNotRequiredAttribute), inherit: true);
    }
}

public static class WorkspaceResolutionApplicationBuilderExtensions
{
    public static IApplicationBuilder UseWorkspaceResolution(this IApplicationBuilder app)
    {
        return app.UseMiddleware<WorkspaceResolutionMiddleware>();
    }
}
