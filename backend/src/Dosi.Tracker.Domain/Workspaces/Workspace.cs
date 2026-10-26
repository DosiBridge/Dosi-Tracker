using System;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Workspaces;

/// <summary>
/// A team's workspace. Deleting one (<see cref="WorkspaceManager.DeleteAsync"/>) is a soft delete: from then on the
/// workspace is gone for everyone (its members are no longer members of it), and once its grace period is over the
/// erasure sweep purges its data — captures and their files, tracked time, projects, people — and records
/// <see cref="PurgedTime"/>. The row itself stays, nameless, so the invoices it was billed keep their reference.
/// </summary>
public class Workspace : FullAuditedAggregateRoot<Guid>
{
    public string Name { get; private set; } = null!;

    /// <summary>Upper-cased name; unique among the workspaces that are not deleted, so names are unique
    /// case-insensitively and a deleted workspace's name can be used again.</summary>
    public string NormalizedName { get; private set; } = null!;

    public bool IsArchived { get; private set; }
    public DateTime? ArchivedTime { get; private set; }
    public Guid? ArchivedBy { get; private set; }

    /// <summary>When the data of the deleted workspace was purged; null while it is not deleted, or not purged yet.</summary>
    public DateTime? PurgedTime { get; private set; }

    protected Workspace()
    {
    }

    public Workspace(Guid id, string name)
        : base(id)
    {
        SetName(name);
    }

    public void SetName(string name)
    {
        Name = Check.NotNullOrWhiteSpace(name, nameof(name), WorkspaceConsts.MaxNameLength).Trim();
        NormalizedName = NormalizeName(Name);
    }

    public static string NormalizeName(string name) => name.Trim().ToUpperInvariant();

    public void Archive(Guid? archivedBy = null, DateTime? archivedTime = null)
    {
        if (IsArchived)
        {
            return; // Idempotent
        }

        IsArchived = true;
        ArchivedBy = archivedBy;
        ArchivedTime = archivedTime;
    }

    public void Restore()
    {
        if (!IsArchived)
        {
            return; // Idempotent
        }

        IsArchived = false;
        ArchivedBy = null;
        ArchivedTime = null;
    }

    /// <summary>The name a purged workspace keeps: it identifies nothing.</summary>
    public static string PurgedName(Guid id) => $"deleted-{id:N}";

    /// <summary>Records that the deleted workspace's data is gone, and drops its name (which may name a person or a
    /// client).</summary>
    public void MarkPurged(DateTime purgedTime)
    {
        if (!IsDeleted)
        {
            throw new InvalidOperationException("Only a deleted workspace can be purged.");
        }

        SetName(PurgedName(Id));
        PurgedTime = purgedTime;
    }
}
