using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Claims;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Http;
using Shouldly;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// In-process integration harness composing actual WorkspaceResolutionMiddleware +
/// real WorkspaceRoleLookup querying SQLite membership + actual production WorkspaceRolePermissionValueProvider.
/// Exercises the real production authorization and admission boundary against synthetic claims without test doubles.
/// </summary>
public abstract class WorkspaceResolutionIntegrationTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid UserA = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    private static readonly Guid UserB = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000003");
    private static readonly Guid DualUser = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000004");

    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly WorkspaceRolePermissionValueProvider _permissionProvider;
    private readonly IPermissionDefinitionManager _permissionDefinitions;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;

    protected WorkspaceResolutionIntegrationTests()
    {
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _permissionProvider = GetRequiredService<WorkspaceRolePermissionValueProvider>();
        _permissionDefinitions = GetRequiredService<IPermissionDefinitionManager>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private static ClaimsPrincipal SyntheticPrincipal(Guid? userId)
    {
        if (userId == null)
        {
            return new ClaimsPrincipal(new ClaimsIdentity());
        }

        return new ClaimsPrincipal(new ClaimsIdentity(
            new List<Claim> { new(AbpClaimTypes.UserId, userId.Value.ToString()) },
            "SyntheticAuth"));
    }

    private DefaultHttpContext CreateHttpContext(string? workspaceHeader, Guid? userId)
    {
        var context = new DefaultHttpContext
        {
            RequestServices = ServiceProvider,
            User = SyntheticPrincipal(userId)
        };
        context.Response.Body = new MemoryStream();

        if (workspaceHeader != null)
        {
            context.Request.Headers[WorkspaceConsts.HeaderName] = workspaceHeader;
        }

        return context;
    }

    private static async Task<(int StatusCode, string? ErrorCode, string? Message)> ReadResponseErrorAsync(HttpContext context)
    {
        context.Response.Body.Position = 0;
        using var reader = new StreamReader(context.Response.Body);
        var text = await reader.ReadToEndAsync();
        if (string.IsNullOrWhiteSpace(text))
        {
            return (context.Response.StatusCode, null, null);
        }

        using var doc = JsonDocument.Parse(text);
        if (doc.RootElement.TryGetProperty("error", out var error))
        {
            var code = error.TryGetProperty("code", out var c) ? c.GetString() : null;
            var message = error.TryGetProperty("message", out var m) ? m.GetString() : null;
            return (context.Response.StatusCode, code, message);
        }

        return (context.Response.StatusCode, null, null);
    }

    private async Task<bool> CheckProductionPermissionGrantedAsync(Guid userId, string permissionName)
    {
        var definition = await _permissionDefinitions.GetAsync(permissionName);
        var context = new PermissionValueCheckContext(definition, SyntheticPrincipal(userId));
        var result = await _permissionProvider.CheckAsync(context);
        return result == PermissionGrantResult.Granted;
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB)> SeedWorkspacesAndMembersAsync()
    {
        if (await _userManager.FindByIdAsync(UserA.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserA, "userA", "userA@acme.test"));
        }
        if (await _userManager.FindByIdAsync(UserB.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserB, "userB", "userB@acme.test"));
        }
        if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(DualUser, "dualUser", "dualUser@acme.test"));
        }

        var wsA = await ServiceProvider.CreateWorkspaceAsync("Integration-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Integration-WS-B");

        // UserA is Owner in Workspace A only
        await ServiceProvider.AddWorkspaceMemberAsync(UserA, WorkspaceRoles.Owner, wsA);

        // UserB is Owner in Workspace B only
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);

        // DualUser is Owner in Workspace A, Member in Workspace B
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsB);

        return (wsA, wsB);
    }

    [Fact]
    public async Task A_Only_Caller_Spoofing_Workspace_B_Is_Rejected_With_403_Before_Downstream()
    {
        var (_, wsB) = await SeedWorkspacesAndMembersAsync();
        var context = CreateHttpContext(wsB.ToString(), UserA);

        var nextCalled = false;
        var middleware = new WorkspaceResolutionMiddleware(_ =>
        {
            nextCalled = true;
            return Task.CompletedTask;
        });

        // A request arrives in no workspace, as in production; outside a change the test harness works in
        // TrackerTestConsts.WorkspaceId (TestCurrentWorkspace), which the null assertion would mistake for a bound one.
        using (_currentWorkspace.Change(null))
        {
            await middleware.InvokeAsync(context);

            nextCalled.ShouldBeFalse("Downstream pipeline must NOT be invoked when caller is not a member of the target workspace");
            var (status, code, message) = await ReadResponseErrorAsync(context);
            status.ShouldBe(StatusCodes.Status403Forbidden);
            code.ShouldBe(TrackerDomainErrorCodes.NotWorkspaceMember);
            message.ShouldBe("You are not a member of this workspace.");
            _currentWorkspace.Id.ShouldBeNull("Ambient workspace must not be populated upon admission denial");
        }
    }

    [Fact]
    public async Task Legitimate_Dual_Member_Owner_In_A_Vs_Member_In_B_Uses_Real_Production_Permissions()
    {
        var (wsA, wsB) = await SeedWorkspacesAndMembersAsync();

        // 1. DualUser invokes under Workspace A header -> resolved as Owner
        var contextA = CreateHttpContext(wsA.ToString(), DualUser);
        var downstreamRanA = false;
        var middlewareA = new WorkspaceResolutionMiddleware(async _ =>
        {
            downstreamRanA = true;
            _currentWorkspace.Id.ShouldBe(wsA);
            _currentWorkspace.Role.ShouldBe(WorkspaceRoles.Owner);

            // Real production WorkspaceRolePermissionValueProvider evaluation under Owner role
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Projects.Create)).ShouldBeTrue();
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Billing.Default)).ShouldBeTrue();
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Projects.Delete)).ShouldBeTrue();
        });

        await middlewareA.InvokeAsync(contextA);
        downstreamRanA.ShouldBeTrue();
        contextA.Response.StatusCode.ShouldBe(StatusCodes.Status200OK);

        // 2. DualUser invokes under Workspace B header -> resolved as Member
        var contextB = CreateHttpContext(wsB.ToString(), DualUser);
        var downstreamRanB = false;
        var middlewareB = new WorkspaceResolutionMiddleware(async _ =>
        {
            downstreamRanB = true;
            _currentWorkspace.Id.ShouldBe(wsB);
            _currentWorkspace.Role.ShouldBe(WorkspaceRoles.Member);

            // Real production WorkspaceRolePermissionValueProvider evaluation under Member role
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Projects.Create)).ShouldBeFalse("Members do not hold Projects.Create");
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Billing.Default)).ShouldBeFalse("Members do not hold Billing.Default");
            (await CheckProductionPermissionGrantedAsync(DualUser, TrackerPermissions.Projects.Delete)).ShouldBeFalse("Members do not hold Projects.Delete");
        });

        await middlewareB.InvokeAsync(contextB);
        downstreamRanB.ShouldBeTrue();
        contextB.Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
    }

    [Fact]
    public async Task Missing_Or_Unknown_Workspace_Membership_Is_Denied_With_403_Before_Downstream()
    {
        await SeedWorkspacesAndMembersAsync();
        var unknownWorkspaceId = _guidGenerator.Create();
        var context = CreateHttpContext(unknownWorkspaceId.ToString(), UserA);

        var nextCalled = false;
        var middleware = new WorkspaceResolutionMiddleware(_ =>
        {
            nextCalled = true;
            return Task.CompletedTask;
        });

        await middleware.InvokeAsync(context);

        nextCalled.ShouldBeFalse();
        var (status, code, _) = await ReadResponseErrorAsync(context);
        status.ShouldBe(StatusCodes.Status403Forbidden);
        code.ShouldBe(TrackerDomainErrorCodes.NotWorkspaceMember);
    }

    [Fact]
    public async Task Empty_Guid_Workspace_Header_Is_Refused_With_400_Validation_Before_Downstream()
    {
        await SeedWorkspacesAndMembersAsync();
        var context = CreateHttpContext(Guid.Empty.ToString(), UserA);

        var nextCalled = false;
        var middleware = new WorkspaceResolutionMiddleware(_ =>
        {
            nextCalled = true;
            return Task.CompletedTask;
        });

        await middleware.InvokeAsync(context);

        nextCalled.ShouldBeFalse("Downstream pipeline must NOT be invoked when workspace header is Guid.Empty");
        var (status, code, _) = await ReadResponseErrorAsync(context);
        status.ShouldBe(StatusCodes.Status400BadRequest);
        code.ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Anonymous_Caller_With_Workspace_Header_Leaves_Workspace_Unset()
    {
        var (wsA, _) = await SeedWorkspacesAndMembersAsync();
        var context = CreateHttpContext(wsA.ToString(), userId: null);

        var nextCalled = false;
        var middleware = new WorkspaceResolutionMiddleware(_ =>
        {
            nextCalled = true;
            _currentWorkspace.Id.ShouldBeNull("Anonymous request must not bind workspace");
            return Task.CompletedTask;
        });

        // In no workspace, as in production (see A_Only_Caller_Spoofing_Workspace_B_Is_Rejected_With_403_Before_Downstream).
        using (_currentWorkspace.Change(null))
        {
            await middleware.InvokeAsync(context);
        }

        nextCalled.ShouldBeTrue();
    }
}
