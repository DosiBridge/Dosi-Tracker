import ApplicationServices
import CoreGraphics
import DosiTrackerCore
import Foundation

/// Whether keyboard and mouse input is being captured, for the menu and the
/// window.
public enum InputCaptureState: Equatable, Sendable {
    /// Not started, or stopped.
    case stopped
    /// Input Monitoring is not granted: activity reads "not measured" until
    /// it is. The tap is tried again every ``InputMonitor/retryInterval``.
    case waitingForPermission
    /// The event tap is delivering input.
    case running
    /// The tap could not be created although Input Monitoring is granted;
    /// tried again every ``InputMonitor/retryInterval``.
    case failed
}

/// Keyboard and mouse input through a listen-only Quartz event tap: the
/// macOS counterpart of the Windows agent's low-level hooks
/// (`tracking/input.rs`), feeding the same ``InputLog`` through the same
/// ``InputFilter`` rules, so both agents count the same presses, clicks and
/// active seconds.
///
/// * The tap listens to key downs and ups, modifier changes, mouse button
///   downs, movement, drags and the scroll wheel (``TapEventMapping``). It
///   records when input happened, never what was typed. A key's auto-repeat
///   (`kCGKeyboardEventAutorepeat`) is not a press; events posted by software
///   (source state other than the HID system's) are counted aside as injected
///   and never as input.
/// * It runs on a thread of its own with its own run loop, so a busy main
///   thread never delays it and it never loads the main thread.
/// * A tap the system disabled (`tapDisabledByTimeout`,
///   `tapDisabledByUserInput`) is enabled again at once.
/// * Without Input Monitoring the tap cannot be created: the capture is
///   marked unavailable (blocks read "not measured", never a misleading 0%),
///   the permission is asked for once (`IOHIDRequestAccess`, unless the app
///   prefers to ask itself), and the tap is tried again every
///   ``retryInterval`` until it can be created.
/// * A watchdog (port of the Windows hook watchdog) recreates a tap that
///   stayed silent while the system saw input, and marks the capture
///   unavailable when the pointer provably moved unseen (``PointerWatch``).
///
/// Start it once at launch and keep it running: a block reads only its own
/// window of the log, so input while paused or signed out never counts.
///
/// Thread-safe: ``start(requestingPermission:)``, ``stop()``, ``state`` and
/// ``setStateHandler(_:)`` may be used from any thread; the tap lives on its
/// own thread.
public final class InputMonitor: @unchecked Sendable {
    /// How often a missing tap is tried again; also the watchdog's check
    /// interval.
    public static let retryInterval: TimeInterval = 10

    private let log: InputLog
    private let filter: InputFilter
    private let lock = NSLock()
    private var current: TapThread?
    /// Tells a stopped tap thread's late reports from the current one's.
    private var generation = 0
    private var currentState: InputCaptureState = .stopped
    private var stateHandler: (@Sendable (InputCaptureState) -> Void)?

    public init(log: InputLog) {
        self.log = log
        self.filter = InputFilter(log: log)
    }

    deinit {
        stop()
    }

    /// The capture's state now.
    public var state: InputCaptureState {
        lock.lock()
        defer { lock.unlock() }
        return currentState
    }

    /// Be told when the state changes. The handler runs on the tap thread
    /// (or the caller of ``stop()``); hop to the main actor for the UI.
    public func setStateHandler(_ handler: (@Sendable (InputCaptureState) -> Void)?) {
        lock.lock()
        stateHandler = handler
        lock.unlock()
    }

    /// Start capturing; calling it again while running does nothing.
    /// - Parameter requestingPermission: ask for Input Monitoring (the system
    ///   prompt, shown once) when it was never asked for. Pass false when the
    ///   app asks itself (``Permissions/requestInputMonitoring()``); the tap
    ///   is then created once the permission is granted.
    public func start(requestingPermission: Bool = true) {
        lock.lock()
        guard current == nil else {
            lock.unlock()
            return
        }
        generation += 1
        let runGeneration = generation
        let tapThread = TapThread(log: log, filter: filter, requestsPermission: requestingPermission) { [weak self] state in
            self?.report(state, generation: runGeneration)
        }
        current = tapThread
        lock.unlock()
        tapThread.start()
    }

    /// Stop capturing (at quit). Input from now on is not recorded.
    public func stop() {
        lock.lock()
        let tapThread = current
        current = nil
        generation += 1
        lock.unlock()
        tapThread?.stop()
        report(.stopped, generation: nil)
    }

    /// A state reported by the tap thread of `runGeneration` (nil: by
    /// ``stop()``); reports of a stopped thread are ignored.
    private func report(_ state: InputCaptureState, generation runGeneration: Int?) {
        lock.lock()
        if let runGeneration = runGeneration, runGeneration != generation {
            lock.unlock()
            return
        }
        guard state != currentState else {
            lock.unlock()
            return
        }
        currentState = state
        let handler = stateHandler
        lock.unlock()
        CoreLog.info("input", "input capture: \(state)")
        handler?(state)
    }
}

/// The event tap's thread: the tap, its run loop, the retry timer and the
/// watchdog's memory.
///
/// Thread-safe: ``start()`` and ``stop()`` may be called from any thread;
/// everything else runs on, and is confined to, its own thread.
private final class TapThread: @unchecked Sendable {
    private let log: InputLog
    private let filter: InputFilter
    private let requestsPermission: Bool
    private let report: @Sendable (InputCaptureState) -> Void
    /// Hardware input only, for the watchdog: did the user really type or
    /// move the mouse while the tap stayed silent?
    private let hardwareIdle = IdleMonitor(stateID: .hidSystemState)

    private let lock = NSLock()
    private var runLoop: CFRunLoop?
    private var stopRequested = false

    // Confined to the thread.
    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    private var lastAttemptMs: Int64 = 0
    private var pointer = PointerWatch()

    init(log: InputLog, filter: InputFilter, requestsPermission: Bool,
         report: @escaping @Sendable (InputCaptureState) -> Void) {
        self.log = log
        self.filter = filter
        self.requestsPermission = requestsPermission
        self.report = report
    }

    func start() {
        let thread = Thread { [self] in
            self.run()
        }
        thread.name = "com.dosibridge.tracker.input-tap"
        // The tap's callback must answer promptly or the system disables it.
        thread.qualityOfService = .userInteractive
        thread.start()
    }

    /// Ask the thread to remove the tap and end.
    func stop() {
        lock.lock()
        stopRequested = true
        let loop = runLoop
        lock.unlock()
        guard let loop = loop else { return }
        // Runs inside the loop even if it has not started running yet.
        CFRunLoopPerformBlock(loop, CFRunLoopMode.defaultMode.rawValue) {
            CFRunLoopStop(CFRunLoopGetCurrent())
        }
        CFRunLoopWakeUp(loop)
    }

    private var isStopRequested: Bool {
        lock.lock()
        defer { lock.unlock() }
        return stopRequested
    }

    private func run() {
        lock.lock()
        runLoop = CFRunLoopGetCurrent()
        let stoppedEarly = stopRequested
        lock.unlock()
        guard !stoppedEarly else { return }

        if requestsPermission && Permissions.inputMonitoringState() == .notDetermined {
            Permissions.requestInputMonitoring()
        }
        let timer = Timer(timeInterval: InputMonitor.retryInterval, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            // This thread's loop never ends, so objects the system calls
            // autorelease are drained per check.
            autoreleasepool {
                self.check()
            }
        }
        RunLoop.current.add(timer, forMode: .common)
        attach()

        while !isStopRequested {
            CFRunLoopRun()
        }
        timer.invalidate()
        detach()
        lock.lock()
        runLoop = nil
        lock.unlock()
    }

    // MARK: The tap (tap thread)

    /// Create the tap and add it to this thread's run loop; on failure mark
    /// the capture unavailable (until the next attempt succeeds and an event
    /// arrives).
    private func attach() {
        let now = EpochTime.milliseconds(Date())
        lastAttemptMs = now
        let access = Permissions.inputMonitoringState()
        guard access != .notDetermined || requestsPermission else {
            // Never asked, and the app asks itself: creating the tap now
            // would show the prompt.
            log.markCaptureUnavailable(sinceMs: now)
            report(.waitingForPermission)
            return
        }
        let userInfo = Unmanaged.passUnretained(self).toOpaque()
        guard let port = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .headInsertEventTap, options: .listenOnly,
                                           eventsOfInterest: CGEventMask(TapEventMapping.eventMask),
                                           callback: inputTapCallback, userInfo: userInfo)
        else {
            log.markCaptureUnavailable(sinceMs: now)
            report(access == .granted ? .failed : .waitingForPermission)
            return
        }
        guard let runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0) else {
            CFMachPortInvalidate(port)
            log.markCaptureUnavailable(sinceMs: now)
            report(.failed)
            return
        }
        CFRunLoopAddSource(CFRunLoopGetCurrent(), runLoopSource, .commonModes)
        CGEvent.tapEnable(tap: port, enable: true)
        tap = port
        source = runLoopSource
        // Keys held across a gap may have been released unseen.
        filter.reset()
        pointer.reset()
        log.captureStarted(atMs: now)
        report(.running)
    }

    private func detach() {
        if let port = tap {
            CGEvent.tapEnable(tap: port, enable: false)
            CFMachPortInvalidate(port)
        }
        if let runLoopSource = source {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), runLoopSource, .commonModes)
        }
        tap = nil
        source = nil
    }

    /// Every ``InputMonitor/retryInterval``: create a missing tap, notice a
    /// withdrawn permission, and run the watchdog.
    private func check() {
        let now = EpochTime.milliseconds(Date())
        guard let port = tap else {
            attach()
            return
        }
        if Permissions.inputMonitoringState() == .denied && !AXIsProcessTrusted() {
            // Turned off in System Settings: the tap no longer hears input.
            CoreLog.warning("input", "Input Monitoring was turned off; input capture waits until it is granted again")
            detach()
            log.markCaptureUnavailable(sinceMs: now)
            report(.waitingForPermission)
            return
        }
        if !CGEvent.tapIsEnabled(tap: port) {
            CGEvent.tapEnable(tap: port, enable: true)
            filter.reset()
        }

        let location = CGEvent(source: nil)?.location
        pointer.observe(x: location.map { Double($0.x) }, y: location.map { Double($0.y) }, atMs: now)
        let lastEvent = log.lastCaptureEventMs
        // Capped at a day: any longer is "long ago" to the watchdog.
        let systemIdleMs = hardwareIdle.secondsSinceLastInput().map { Int64((min($0, 86_400) * 1000).rounded()) }
        guard InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: lastEvent,
                                                systemIdleMs: systemIdleMs, lastAttemptMs: lastAttemptMs)
        else {
            return
        }
        if pointer.missedInput(lastCaptureEventMs: lastEvent, checkIntervalMs: Int64(InputMonitor.retryInterval * 1000)) {
            CoreLog.warning("input", "the event tap missed the user's input; recreating it")
            log.markCaptureUnavailable(sinceMs: lastEvent)
        } else {
            // Most likely input it legitimately cannot see (a secure text
            // field); recreating it is cheap and rules out a dead tap.
            CoreLog.debug("input", "the system saw input the event tap did not; recreating it")
        }
        detach()
        attach()
    }

    /// One event from the tap (tap thread). Kept minimal: the system disables
    /// a tap whose callback is slow.
    fileprivate func handle(_ type: CGEventType, _ event: CGEvent) {
        switch type {
        case .tapDisabledByTimeout, .tapDisabledByUserInput:
            if let port = tap {
                CGEvent.tapEnable(tap: port, enable: true)
            }
            // Key-ups may have been missed while it was off.
            filter.reset()
            return
        default:
            break
        }
        let isKey = type == .keyDown || type == .keyUp || type == .flagsChanged
        let inputs = TapEventMapping.inputs(
            eventType: type.rawValue,
            keyCode: isKey ? Int(event.getIntegerValueField(.keyboardEventKeycode)) : 0,
            flags: event.flags.rawValue,
            autorepeat: type == .keyDown && event.getIntegerValueField(.keyboardEventAutorepeat) != 0
        )
        guard !inputs.isEmpty else { return }
        let injected = TapEventMapping.isInjected(sourceStateID: event.getIntegerValueField(.eventSourceStateID))
        let ms = EpochTime.milliseconds(Date())
        for input in inputs {
            filter.apply(input, injected: injected, atMs: ms)
        }
    }
}

/// The tap's C callback: hands the event to its ``TapThread`` (the user info).
/// A listen-only tap passes every event on unchanged.
private func inputTapCallback(proxy: CGEventTapProxy, type: CGEventType, event: CGEvent,
                              userInfo: UnsafeMutableRawPointer?) -> Unmanaged<CGEvent>? {
    if let userInfo = userInfo {
        Unmanaged<TapThread>.fromOpaque(userInfo).takeUnretainedValue().handle(type, event)
    }
    return Unmanaged.passUnretained(event)
}
