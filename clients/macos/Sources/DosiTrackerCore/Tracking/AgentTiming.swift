import Foundation

/// Timing rules shared by the worker and the UI (CONTRACT §1-§4; the Windows
/// agent uses the same values).
public enum AgentTiming {
    /// A block shorter than this is not uploaded: pausing, switching project,
    /// going idle or quitting drops it instead of recording a seconds-long
    /// block scored 100% from a single click; "Sync now" leaves it running.
    public static let minimumBlock: TimeInterval = 60

    /// How often the worker ticks while running: idle detection, the
    /// sleep-gap check and the capture plan are evaluated on each tick. Far
    /// below ``SleepGapDetector/threshold`` so a missed sleep is told apart
    /// from a slow tick.
    public static let tickInterval: TimeInterval = 5

    /// How often a signed-in agent without a session (server unreachable, no
    /// project yet) tries again.
    public static let reconnectInterval: TimeInterval = 30

    /// How often the project list (interval, permissions, membership) is
    /// refreshed while signed in.
    public static let projectRefreshInterval: TimeInterval = 5 * 60

    /// Heartbeat period while running and signed in.
    public static let heartbeatInterval: TimeInterval = 60

    /// Budget of the best-effort `stopped` / `signedout` heartbeat.
    public static let finalHeartbeatTimeout: TimeInterval = 2

    /// Parked (rejected) rows, and other accounts' queued rows, are kept this
    /// long for diagnostics before the queue reclaims them.
    public static let rejectedRetentionDays = 14

    /// Rows read from the queue per round trip (each can embed a screenshot).
    public static let uploadBatchSize = 20

    /// Upper bound on uploads per sync pass, so a large offline backlog drains
    /// over a few passes instead of starving everything else.
    public static let maxUploadsPerPass = 100

    /// How long the earlier days of the week chart are trusted before being
    /// fetched again (a late offline backlog can still change them).
    public static let weekHistoryTTL: TimeInterval = 30 * 60

    /// Time allowed to save the running block when the Mac shuts down or the
    /// session logs out (local enqueue only, no network).
    public static let shutdownSaveBudget: TimeInterval = 2

    /// Time allowed to upload queued blocks when the user quits.
    public static let quitSyncBudget: TimeInterval = 20
}
