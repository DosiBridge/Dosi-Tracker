using System;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Deterministic keyset pagination cursor for activity retention cleanup (REQ-DAT-01).
/// Orders by (StartedAt, ActivityId) to guarantee stable forward progress past preserved dependencies.
/// </summary>
public record ActivityRetentionCursor(DateTime StartedAt, Guid ActivityId);
