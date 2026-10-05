using System.Linq;
using Dosi.Tracker.Routes;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The HTTP routes ABP derives from <see cref="TeamAppService"/>, against the shared contract (and the web app's route
/// builders). ABP puts a method's only "...Id" parameter into the path (<c>/reset-password/{userId}</c>) unless the
/// parameter is optional; the contract, however, passes <c>userId</c> in the query string.
/// </summary>
public class TeamRoutesTests
{
    private static (string Verb, string Route) RouteOf(string methodName)
    {
        return ConventionalRoutes.Of(typeof(TeamAppService), methodName);
    }

    [Theory]
    [InlineData(nameof(TeamAppService.ResetPasswordAsync), "POST", "api/app/team/reset-password")]
    [InlineData(nameof(TeamAppService.RemoveFromWorkspaceAsync), "DELETE", "api/app/team/from-workspace")]
    [InlineData(nameof(TeamAppService.UpdateMemberAsync), "PUT", "api/app/team/member")]
    [InlineData(nameof(TeamAppService.AddMemberAsync), "POST", "api/app/team/member")]
    [InlineData(nameof(TeamAppService.InviteMemberAsync), "POST", "api/app/team/invite-member")]
    [InlineData(nameof(TeamAppService.GetMembersAsync), "GET", "api/app/team/members")]
    [InlineData(nameof(TeamAppService.GetProjectMembersAsync), "GET", "api/app/team/project-members/{projectId}")]
    public void Team_Endpoints_Should_Have_The_Contract_Routes(string methodName, string verb, string route)
    {
        RouteOf(methodName).ShouldBe((verb, route));
    }

    [Fact]
    public void The_User_Id_Of_Reset_And_Removal_Should_Stay_In_The_Query_String()
    {
        // The escape hatch ABP honours: an optional parameter is never turned into a route segment.
        foreach (var methodName in new[] { nameof(TeamAppService.ResetPasswordAsync), nameof(TeamAppService.RemoveFromWorkspaceAsync) })
        {
            var userId = typeof(TeamAppService).GetMethod(methodName)!.GetParameters().Single();
            userId.Name.ShouldBe("userId");
            userId.IsOptional.ShouldBeTrue();
        }
    }
}
