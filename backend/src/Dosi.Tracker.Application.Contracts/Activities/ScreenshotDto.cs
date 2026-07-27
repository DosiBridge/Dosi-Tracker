using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Activities;

public class ScreenshotDto : CreationAuditedEntityDto<Guid>
{
    public Guid ActivityId { get; set; }
    public string Kind { get; set; } // "screen" or "webcam"
    public bool Blurred { get; set; }
    public DateTime CapturedAt { get; set; }
    public long SizeBytes { get; set; }
    public string ContentType { get; set; }
}

/// <summary>Raw capture bytes; streamed as a file by the screenshot content controller.</summary>
public class ScreenshotContentDto
{
    public byte[] Bytes { get; set; }
    public string ContentType { get; set; }
    public string FileName { get; set; }
}
