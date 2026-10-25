import Foundation

/// Where a recorded block is on its way to the server.
public enum UploadState: String, Equatable, Sendable {
    /// Saved locally, waiting to upload.
    case queued
    case uploaded
    /// The server refused it; it is kept locally but never uploaded.
    case rejected
}

/// A block recorded during this run, for the window's recent-activity list.
public struct RecentBlock: Equatable, Sendable, Identifiable {
    /// The queue row, so the upload result can be matched to it.
    public let rowId: Int64
    public var projectTitle: String
    public var startedAt: Date
    public var endedAt: Date
    /// Nil when activity was not measured.
    public var productivity: Int?
    /// Nil when that kind of input was not measured.
    public var keyboardHits: Int?
    public var mouseClicks: Int?
    public var hasScreenshot: Bool
    public var endReason: EndReason?
    public var upload: UploadState
    /// The server's reason, for a rejected block.
    public var message: String?

    public var id: Int64 { rowId }

    public init(rowId: Int64, projectTitle: String, startedAt: Date, endedAt: Date, productivity: Int?,
                keyboardHits: Int?, mouseClicks: Int?, hasScreenshot: Bool, endReason: EndReason?,
                upload: UploadState = .queued, message: String? = nil) {
        self.rowId = rowId
        self.projectTitle = projectTitle
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.productivity = productivity
        self.keyboardHits = keyboardHits
        self.mouseClicks = mouseClicks
        self.hasScreenshot = hasScreenshot
        self.endReason = endReason
        self.upload = upload
        self.message = message
    }

    /// The summary of a block just queued as `rowId`.
    public init(rowId: Int64, projectTitle: String, payload: ActivityPayload) {
        self.init(
            rowId: rowId,
            projectTitle: projectTitle,
            startedAt: payload.startedAt,
            endedAt: payload.endedAt,
            productivity: payload.productivity,
            keyboardHits: payload.keyboardHits,
            mouseClicks: payload.mouseClicks,
            hasScreenshot: payload.screenshotPngBase64 != nil,
            endReason: payload.endReason
        )
    }
}

/// Recent blocks, newest first, bounded to ``limit`` (port of the Windows
/// worker's `push_recent_block` / `set_upload_state`).
public struct RecentBlocks: Equatable, Sendable {
    public static let limit = 12

    public private(set) var items: [RecentBlock] = []

    public init() {}

    public mutating func push(_ block: RecentBlock) {
        items.insert(block, at: 0)
        if items.count > RecentBlocks.limit {
            items.removeLast(items.count - RecentBlocks.limit)
        }
    }

    /// Record where a queued block is now; ignored if it is not listed.
    public mutating func setUploadState(rowId: Int64, _ state: UploadState, message: String? = nil) {
        guard let index = items.firstIndex(where: { $0.rowId == rowId }) else { return }
        items[index].upload = state
        items[index].message = message
    }

    /// Apply a sync pass: uploaded rows are marked uploaded, parked ones
    /// rejected with the server's reason.
    public mutating func apply(_ report: SyncReport) {
        for id in report.uploaded {
            setUploadState(rowId: id, .uploaded)
        }
        for parked in report.parked {
            setUploadState(rowId: parked.id, .rejected, message: parked.reason)
        }
    }

    /// The next account must not see this one's work.
    public mutating func removeAll() {
        items.removeAll()
    }
}
