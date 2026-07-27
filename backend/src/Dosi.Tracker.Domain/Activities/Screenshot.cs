using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Activities;

public class Screenshot : CreationAuditedEntity<Guid>, IMultiTenant
{
    public const string ScreenKind = "screen";
    public const string WebcamKind = "webcam";

    public Guid? TenantId { get; set; }
    public Guid ActivityId { get; set; }
    public string Kind { get; set; } // "screen" or "webcam"
    public string StorageUrl { get; set; } // Blob name (S3/R2 key or database blob name)
    public bool Blurred { get; set; }
    public DateTime CapturedAt { get; set; }
    public long SizeBytes { get; set; }
    public string ContentType { get; set; }

    protected Screenshot()
    {
    }

    public Screenshot(
        Guid id,
        Guid? tenantId,
        Guid activityId,
        string storageUrl,
        DateTime capturedAt,
        long sizeBytes,
        string contentType = "image/jpeg",
        string kind = ScreenKind)
    {
        Id = id;
        TenantId = tenantId;
        ActivityId = activityId;
        StorageUrl = storageUrl;
        CapturedAt = capturedAt;
        SizeBytes = sizeBytes;
        ContentType = contentType;
        Kind = kind;
    }
}
