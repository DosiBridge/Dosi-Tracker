using System;
using System.Reflection;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Platform;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Routes;

/// <summary>
/// The AI assist routes the web app is built against (frozen), as ABP derives them from the application services, and
/// how each service is guarded: AI assist needs a signed-in member of the workspace in <c>X-Workspace-Id</c>; the
/// platform part needs the platform permission and no workspace.
/// </summary>
public class AiAssistRoutesTests
{
    [Theory]
    [InlineData(typeof(AiAssistAppService), nameof(AiAssistAppService.GetStatusAsync), "GET", "api/app/ai-assist/status")]
    [InlineData(typeof(AiAssistAppService), nameof(AiAssistAppService.GetSettingsAsync), "GET", "api/app/ai-assist/settings")]
    [InlineData(typeof(AiAssistAppService), nameof(AiAssistAppService.UpdateSettingsAsync), "PUT", "api/app/ai-assist/settings")]
    [InlineData(typeof(AiAssistAppService), nameof(AiAssistAppService.GetUsageAsync), "GET", "api/app/ai-assist/usage")]
    [InlineData(typeof(PlatformAiAppService), nameof(PlatformAiAppService.GetOverviewAsync), "GET", "api/app/platform-ai/overview")]
    [InlineData(typeof(PlatformAiAppService), nameof(PlatformAiAppService.UpdateStatusAsync), "PUT", "api/app/platform-ai/status")]
    public void AI_Endpoints_Should_Have_The_Contract_Routes(Type appService, string methodName, string verb, string route)
    {
        ConventionalRoutes.Of(appService, methodName).ShouldBe((verb, route));
    }

    [Fact]
    public void AI_Assist_Should_Need_A_Signed_In_Caller_In_A_Workspace()
    {
        typeof(AiAssistAppService).GetCustomAttribute<AuthorizeAttribute>().ShouldNotBeNull().Policy.ShouldBeNull();
        typeof(AiAssistAppService).GetCustomAttribute<WorkspaceNotRequiredAttribute>().ShouldBeNull();
    }

    [Fact]
    public void Platform_AI_Should_Need_The_Platform_Permission_And_No_Workspace()
    {
        typeof(PlatformAiAppService).GetCustomAttribute<AuthorizeAttribute>().ShouldNotBeNull()
            .Policy.ShouldBe(TrackerPermissions.Platform.Default);
        typeof(PlatformAiAppService).GetCustomAttribute<WorkspaceNotRequiredAttribute>().ShouldNotBeNull();
    }
}
