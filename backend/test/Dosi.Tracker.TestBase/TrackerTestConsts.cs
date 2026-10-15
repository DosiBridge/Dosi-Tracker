using System;

namespace Dosi.Tracker;

public static class TrackerTestConsts
{
    public const string CollectionDefinitionName = "Tracker collection";

    /// <summary>The workspace tests work in unless they choose another (seeded by the domain test module).</summary>
    public static readonly Guid WorkspaceId = Guid.Parse("7b0c5e1e-4f6a-4d39-9d0c-1f3c2b9a7e01");

    public const string WorkspaceName = "Test Workspace";
}
