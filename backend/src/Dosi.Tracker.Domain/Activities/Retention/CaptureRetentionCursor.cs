using System;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// REQ-DAT-01: Keyset pagination cursor for capture retention.
/// Points to the last processed candidate in the deterministic (CapturedAt, Id) order.
/// Enables deterministic continuation past preserved poison or temporarily failing rows.
/// </summary>
public sealed record CaptureRetentionCursor(DateTime CapturedAt, Guid ScreenshotId);
