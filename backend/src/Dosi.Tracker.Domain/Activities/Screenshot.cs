using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Activities;

public class Screenshot : CreationAuditedEntity<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public Guid ActivityId { get; set; }
    public string StorageUrl { get; set; } // Cloudflare R2 key/URL
    public bool Blurred { get; set; }
    public DateTime CapturedAt { get; set; }
    public long SizeBytes { get; set; }
    public string ContentType { get; set; }

    protected Screenshot()
    {
    }

    public Screenshot(Guid id, Guid? tenantId, Guid activityId, string storageUrl, DateTime capturedAt, long sizeBytes, string contentType = "image/jpeg")
    {
        Id = id;
        TenantId = tenantId;
        ActivityId = activityId;
        StorageUrl = storageUrl;
        CapturedAt = capturedAt;
        SizeBytes = sizeBytes;
        ContentType = contentType;
    }
}
