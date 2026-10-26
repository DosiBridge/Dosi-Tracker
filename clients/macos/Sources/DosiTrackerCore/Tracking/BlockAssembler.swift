import Foundation

/// One block to record: its project and capture policy, its time window, why
/// it ended, and the task note.
public struct BlockSpec: Equatable, Sendable {
    public var projectId: String
    /// The session's effective permissions.
    public var permissions: CapturePermissions
    /// Screen captures must be blurred: an unblurred one is never sent.
    public var blurScreenshots: Bool
    public var startedAt: Date
    public var endedAt: Date
    /// Nil for a cut by the interval timer.
    public var endReason: EndReason?
    /// The task note ("What are you working on?").
    public var taskNote: String?

    public init(projectId: String, permissions: CapturePermissions, blurScreenshots: Bool, startedAt: Date,
                endedAt: Date, endReason: EndReason?, taskNote: String?) {
        self.projectId = projectId
        self.permissions = permissions
        self.blurScreenshots = blurScreenshots
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.endReason = endReason
        self.taskNote = taskNote
    }

    /// The block of `session` over `span`.
    public init(session: TrackingSession, span: DateInterval, endReason: EndReason?, taskNote: String?) {
        self.init(projectId: session.projectId, permissions: session.permissions,
                  blurScreenshots: session.blurScreenshots, startedAt: span.start, endedAt: span.end,
                  endReason: endReason, taskNote: taskNote)
    }

    /// May the block take a capture now that it ends? Not when the Mac is
    /// locked, asleep, idle or shutting down.
    public var capturesAtEnd: Bool { endReason?.allowsCaptureAtEnd ?? true }
}

/// One screen capture, encoded for upload (JPEG preferred, PNG accepted).
public struct ScreenCapture: Equatable, Sendable {
    /// The screen, at most ``CaptureLimits/maxScreenWidth`` wide.
    public var image: Data
    /// The list thumbnail, at most ``CaptureLimits/maxThumbnailWidth`` wide.
    /// A thumbnail failure never costs the capture itself.
    public var thumbnail: Data?
    /// When the screen was captured.
    public var capturedAt: Date
    /// The capture and its thumbnail were blurred.
    public var blurred: Bool

    public init(image: Data, thumbnail: Data?, capturedAt: Date, blurred: Bool) {
        self.image = image
        self.thumbnail = thumbnail
        self.capturedAt = capturedAt
        self.blurred = blurred
    }
}

/// What the tracker measured for a block — read from the logs and the system
/// by ``BlockAssembler/measure(_:input:usage:windowNow:runningPrograms:screen:webcamJpeg:)``,
/// turned into the payload by ``BlockAssembler/build(_:measured:device:zone:)``.
public struct Measurements: Equatable, Sendable {
    /// Unix ms of key presses (empty when the keyboard is not recorded).
    public var keyPresses: [Int64]
    /// Unix ms of clicks (empty when the mouse is not recorded).
    public var clicks: [Int64]
    /// Unix seconds with input of the recorded kinds.
    public var activeSeconds: [Int64]
    /// Injected events of the recorded kinds.
    public var injected: Int
    /// The input capture worked for the whole block.
    public var inputAvailable: Bool
    /// Foreground usage (empty when the active window is not recorded).
    public var slices: [UsageSlice]
    /// The window in front at the end of the block.
    public var windowNow: ForegroundWindow?
    /// Programs with a window open, names only (empty when not recorded).
    public var runningPrograms: [WindowInfo]
    public var screen: ScreenCapture?
    public var webcamJpeg: Data?

    public init(keyPresses: [Int64] = [], clicks: [Int64] = [], activeSeconds: [Int64] = [], injected: Int = 0,
                inputAvailable: Bool = true, slices: [UsageSlice] = [], windowNow: ForegroundWindow? = nil,
                runningPrograms: [WindowInfo] = [], screen: ScreenCapture? = nil, webcamJpeg: Data? = nil) {
        self.keyPresses = keyPresses
        self.clicks = clicks
        self.activeSeconds = activeSeconds
        self.injected = injected
        self.inputAvailable = inputAvailable
        self.slices = slices
        self.windowNow = windowNow
        self.runningPrograms = runningPrograms
        self.screen = screen
        self.webcamJpeg = webcamJpeg
    }
}

/// The device's time zone when a block started.
public struct BlockZone: Equatable, Sendable {
    /// IANA id, e.g. `Asia/Dhaka`.
    public var timeZone: String?
    /// The UTC offset at the block's start, in minutes (+360 for Dhaka).
    public var utcOffsetMinutes: Int

    public init(timeZone: String?, utcOffsetMinutes: Int) {
        self.timeZone = timeZone
        self.utcOffsetMinutes = utcOffsetMinutes
    }

    /// `zone` at `date` (its offset follows daylight saving time).
    public init(_ zone: TimeZone, at date: Date) {
        self.init(timeZone: DeviceInfo.wireTimeZone(zone.identifier),
                  utcOffsetMinutes: DeviceInfo.utcOffsetMinutes(zone, at: date))
    }
}

/// Builds the upload payload of a block (port of the Windows agent's
/// `tracking/mod.rs`: `Tracker::snapshot`, `build_activity`,
/// `kept_idle_blocks`), so both platforms send the same numbers for the same
/// input.
public enum BlockAssembler {
    /// Most running programs sent per block (the server's list limit).
    public static let maxRunningPrograms = 200
    /// The longest task note the server stores.
    public static let maxDescriptionLength = 2048

    /// Read everything a block needs from the logs, over its own window only,
    /// so input and usage from before it (while paused, signed out, idle, or
    /// on another project) can never be attributed to it. The stretch since
    /// the last usage sample is credited to `windowNow` first, so usage runs
    /// right up to the end. Only allowed kinds are read; captures the block
    /// may not have are left out.
    public static func measure(_ block: BlockSpec, input: InputLog, usage: UsageRecorder, windowNow: ForegroundWindow?,
                               runningPrograms: [String], screen: ScreenCapture?, webcamJpeg: Data?) -> Measurements {
        let permissions = block.permissions
        let startMs = EpochTime.milliseconds(block.startedAt)
        let endMs = EpochTime.milliseconds(block.endedAt)
        usage.record(nowMs: endMs, window: windowNow)

        return Measurements(
            keyPresses: permissions.keyboard ? input.keyPresses(fromMs: startMs, toMs: endMs) : [],
            clicks: permissions.mouse ? input.clicks(fromMs: startMs, toMs: endMs) : [],
            activeSeconds: input.activeSeconds(from: block.startedAt, to: block.endedAt,
                                               keyboard: permissions.keyboard, mouse: permissions.mouse),
            injected: input.injectedCount(from: block.startedAt, to: block.endedAt,
                                          keyboard: permissions.keyboard, mouse: permissions.mouse),
            inputAvailable: input.isCaptureAvailable(fromMs: startMs, toMs: endMs),
            slices: permissions.activeWindow ? usage.slices(fromMs: startMs, toMs: endMs) : [],
            windowNow: windowNow,
            runningPrograms: permissions.runningPrograms ? runningProgramRows(runningPrograms) : [],
            screen: permissions.screenshot ? screen : nil,
            webcamJpeg: permissions.webcam ? webcamJpeg : nil
        )
    }

    /// Assemble the upload payload of a block (CONTRACT §1).
    ///
    /// Activity is "not measured" — `productivity`, `idleSeconds` and
    /// `injectedInputEvents` left out, and no per-minute timeline (whose
    /// `active: false` minutes would read as idle) — when the block records
    /// neither keyboard nor mouse, or the input capture was not working for
    /// the whole block. Each count is measured on its own: `keyboardHits`
    /// (in the block, each minute and each window) is nil unless the keyboard
    /// is recorded and the capture worked, and `mouseClicks` likewise — never
    /// a 0 for input nobody measured. A capture is attached only when the
    /// project allows it, blurred when it must be (the server would drop an
    /// unblurred one), and within the server's size limits.
    public static func build(_ block: BlockSpec, measured: Measurements, device: DeviceInfo, zone: BlockZone) -> ActivityPayload {
        let permissions = block.permissions
        let start = EpochTime.seconds(block.startedAt)
        let end = EpochTime.seconds(block.endedAt)
        let keyboardMeasured = permissions.keyboard && measured.inputAvailable
        let mouseMeasured = permissions.mouse && measured.inputAvailable
        let inputMeasured = keyboardMeasured || mouseMeasured

        let activeWindows: [WindowInfo]
        if !permissions.activeWindow {
            activeWindows = []
        } else if measured.slices.isEmpty {
            // Too short for a single sample: fall back to the window in front.
            if let front = measured.windowNow, !front.appName.isEmpty {
                activeWindows = [WindowInfo(appName: front.appName, windowTitle: front.windowTitle)]
            } else {
                activeWindows = []
            }
        } else {
            var rows = UsageRecorder.appUsage(measured.slices, keyPresses: measured.keyPresses, clicks: measured.clicks)
            for index in rows.indices {
                if !keyboardMeasured { rows[index].keyboardHits = nil }
                if !mouseMeasured { rows[index].mouseClicks = nil }
            }
            activeWindows = rows
        }

        var timeline: [ActivityMinute] = []
        if inputMeasured {
            timeline = ActivityMath.minuteTimeline(start: start, end: end, slices: measured.slices,
                                                   keyPresses: measured.keyPresses, clicks: measured.clicks,
                                                   activeSeconds: measured.activeSeconds)
            for index in timeline.indices {
                if !keyboardMeasured { timeline[index].keyboardHits = nil }
                if !mouseMeasured { timeline[index].mouseClicks = nil }
            }
        }

        var payload = ActivityPayload(
            projectId: block.projectId,
            startedAt: block.startedAt,
            endedAt: block.endedAt,
            productivity: inputMeasured
                ? ActivityMath.activityPercent(activeSeconds: measured.activeSeconds, start: start, end: end)
                : nil,
            mouseClicks: mouseMeasured ? measured.clicks.count : nil,
            keyboardHits: keyboardMeasured ? measured.keyPresses.count : nil,
            description: cleanDescription(block.taskNote),
            activeWindows: activeWindows,
            runningPrograms: permissions.runningPrograms ? Array(measured.runningPrograms.prefix(maxRunningPrograms)) : [],
            timeline: timeline
        )
        payload.idleSeconds = inputMeasured ? ActivityMath.idleSeconds(timeline: timeline, start: start, end: end) : nil
        payload.injectedInputEvents = inputMeasured ? min(max(measured.injected, 0), Int(Int32.max)) : nil
        payload.endReason = block.endReason

        if permissions.screenshot, let screen = measured.screen, screen.blurred || !block.blurScreenshots {
            payload.attachScreen(image: screen.image, thumbnail: screen.thumbnail,
                                 capturedAt: screen.capturedAt, blurred: screen.blurred)
        }
        if permissions.webcam, let webcam = measured.webcamJpeg {
            payload.attachWebcam(jpeg: webcam)
        }
        payload.applyDeviceContext(device, zone: zone)
        return payload
    }

    /// The away span kept as work after the idle prompt (CONTRACT §4): blocks
    /// of at most one interval each (as equal as possible), 0% active and idle
    /// for their whole length, ended by `idle`, without windows, timeline or
    /// captures, carrying the task note. Nothing for a span under a minute.
    public static func keptIdleBlocks(projectId: String, span: AwaySpan, interval: TimeInterval, description: String?,
                                      device: DeviceInfo, timeZone: TimeZone = .autoupdatingCurrent) -> [ActivityPayload] {
        span.keptBlocks(interval: interval).map { piece in
            var payload = ActivityPayload(
                projectId: projectId,
                startedAt: piece.start,
                endedAt: piece.end,
                productivity: 0,
                description: cleanDescription(description)
            )
            // Whole seconds, truncated, as on Windows.
            let lengthMs = EpochTime.milliseconds(piece.end) - EpochTime.milliseconds(piece.start)
            payload.idleSeconds = Int(min(max(lengthMs, 0) / 1000, Int64(Int32.max)))
            payload.endReason = .idle
            payload.applyDeviceContext(device, zone: BlockZone(timeZone, at: piece.start))
            return payload
        }
    }

    /// Running programs as rows (port of the Windows `program_list`): app
    /// names only — never their window titles (the titles of background
    /// windows are not the worker's focus and can hold document names or
    /// e-mail subjects) — one entry per app, compared case-insensitively, in
    /// the order given, without unnamed entries, at most
    /// ``maxRunningPrograms``.
    public static func runningProgramRows(_ names: [String]) -> [WindowInfo] {
        var seen = Set<String>()
        var rows: [WindowInfo] = []
        for name in names {
            guard !name.isEmpty, seen.insert(name.lowercased()).inserted else { continue }
            rows.append(WindowInfo(appName: name, windowTitle: ""))
            if rows.count == maxRunningPrograms { break }
        }
        return rows
    }

    /// The task note as sent: line breaks and runs of whitespace become single
    /// spaces, other control characters are removed, the ends are trimmed
    /// (the clean-up the server applies to edited notes, CONTRACT §5), and the
    /// result is cut to the server's 2048 UTF-16 units; nil when nothing is
    /// left.
    public static func cleanDescription(_ note: String?) -> String? {
        guard let note = note else { return nil }
        var cleaned = ""
        var pendingSpace = false
        for scalar in note.unicodeScalars {
            if CharacterSet.whitespacesAndNewlines.contains(scalar) {
                pendingSpace = !cleaned.isEmpty
                continue
            }
            if CharacterSet.controlCharacters.contains(scalar) {
                continue
            }
            if pendingSpace {
                cleaned.append(" ")
                pendingSpace = false
            }
            cleaned.unicodeScalars.append(scalar)
        }
        let limited = cleaned.truncatedUTF16(maxDescriptionLength).trimmingCharacters(in: .whitespacesAndNewlines)
        return limited.isEmpty ? nil : limited
    }
}
