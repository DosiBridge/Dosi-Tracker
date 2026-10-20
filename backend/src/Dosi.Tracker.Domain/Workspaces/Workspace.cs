using System;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Workspaces;

public class Workspace : AuditedAggregateRoot<Guid>
{
    public string Name { get; private set; } = null!;

    /// <summary>Upper-cased name; unique, so names are unique case-insensitively.</summary>
    public string NormalizedName { get; private set; } = null!;

    public bool IsArchived { get; private set; }
    public DateTime? ArchivedTime { get; private set; }
    public Guid? ArchivedBy { get; private set; }

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
}
