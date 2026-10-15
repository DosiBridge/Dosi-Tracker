using System;

namespace Dosi.Tracker.Workspaces;

/// <summary>Data that belongs to one workspace. Queries only see the current workspace's rows (a data filter that
/// platform code can disable), and a new row gets the current workspace when it has none.</summary>
public interface IWorkspaceScoped
{
    Guid WorkspaceId { get; }
}
