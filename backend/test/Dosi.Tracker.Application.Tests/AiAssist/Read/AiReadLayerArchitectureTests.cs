using System;
using System.Linq;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Timing;
using Volo.Abp.Users;
using Xunit;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>
/// The read layer may only read what the caller could read through the API: its types depend on app-service interfaces,
/// the extracted note and task rules, workspace membership and settings, the current user and workspace, the clock,
/// and each other. Never on a repository, a queryable or a DbContext.
/// </summary>
public class AiReadLayerArchitectureTests
{
    private static readonly Type[] Allowed =
    {
        typeof(ActivityNoteRules), typeof(ProjectTaskRules), typeof(WorkspaceMembershipManager), typeof(WorkspaceSettingsManager),
        typeof(ICurrentUser), typeof(ICurrentWorkspace), typeof(IClock)
    };

    private static readonly string ReadNamespace = typeof(IAiReadLayer).Namespace!;

    private static bool IsAllowed(Type type) =>
        Allowed.Contains(type) ||
        type.Namespace == ReadNamespace ||
        (type.IsInterface && type.Name.StartsWith('I') && type.Name.EndsWith("AppService", StringComparison.Ordinal) &&
         type.Namespace?.StartsWith("Dosi.Tracker", StringComparison.Ordinal) == true);

    [Fact]
    public void Read_Layer_Types_Should_Depend_Only_On_App_Services_And_The_Allowed_Rules()
    {
        // The services of the namespace (its read models are plain records).
        var types = typeof(IAiReadLayer).Assembly.GetTypes()
            .Where(t => t.Namespace == ReadNamespace && t.IsClass &&
                        (typeof(ITransientDependency).IsAssignableFrom(t) || typeof(ISingletonDependency).IsAssignableFrom(t) ||
                         typeof(IScopedDependency).IsAssignableFrom(t)))
            .ToList();
        types.ShouldContain(typeof(AiReadLayer));
        types.ShouldContain(typeof(AiScopeGuard));

        foreach (var type in types)
        {
            foreach (var parameter in type.GetConstructors().SelectMany(c => c.GetParameters()))
            {
                IsAllowed(parameter.ParameterType).ShouldBeTrue(
                    $"{type.Name} depends on {parameter.ParameterType.Name}, which the read layer may not use.");
            }
        }
    }

    [Fact]
    public void No_Type_Of_The_Read_Layer_Should_Hold_A_Repository_Or_A_Database_Context()
    {
        foreach (var type in typeof(IAiReadLayer).Assembly.GetTypes().Where(t => t.Namespace == ReadNamespace))
        {
            var held = type.GetFields(System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.Static |
                                      System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic)
                .Select(f => f.FieldType)
                .Concat(type.GetConstructors().SelectMany(c => c.GetParameters()).Select(p => p.ParameterType));

            held.ShouldNotContain(t => (t.IsGenericType && (t.GetGenericTypeDefinition() == typeof(IRepository<,>) ||
                                                             t.GetGenericTypeDefinition() == typeof(IReadOnlyRepository<,>))) ||
                                       t.Name.Contains("DbContext") || t.Name.Contains("Repository"),
                type.Name);
        }
    }

    [Fact]
    public void The_Read_Layer_Should_Read_Through_The_Products_App_Services()
    {
        var dependencies = typeof(AiReadLayer).GetConstructors().Single().GetParameters().Select(p => p.ParameterType).ToList();

        dependencies.ShouldContain(typeof(Reporting.IReportingAppService));
        dependencies.ShouldContain(typeof(IActivityAppService));
        dependencies.ShouldContain(typeof(Teams.ITeamAppService));
        dependencies.ShouldContain(typeof(Projects.IProjectAppService));
        dependencies.ShouldContain(typeof(Projects.IProjectTaskAppService));
        dependencies.ShouldContain(typeof(ActivityNoteRules));
        dependencies.ShouldNotContain(t => t.IsGenericType && t.GetGenericTypeDefinition() == typeof(IRepository<,>));
        dependencies.ShouldNotContain(t => t.Name.Contains("DbContext") || t.Name.Contains("Queryable") || t.Name.Contains("Repository"));
    }
}
