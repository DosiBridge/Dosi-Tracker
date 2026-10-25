import AppKit

// The menu-bar icon (the macOS counterpart of the Windows tray icon): the
// brand ring drawn in code as a template image — so it follows the menu bar's
// light, dark and tinted appearances — with a centre glyph per state, since a
// template image carries no colour.

/// Which glyph the icon wears.
enum StatusBadge: Equatable {
    /// Recording: the filled dot of the brand mark.
    case tracking
    /// Paused or on a break: pause bars.
    case paused
    /// Away, locked or asleep: a hollow dot.
    case away
    /// Connecting, offline, or no project chosen: a dashed ring.
    case waiting
    /// Signed out: the bare ring.
    case signedOut
    /// Needs the user (sign-in expired, stopped): an exclamation mark.
    case attention

    static func of(_ status: TrackerStatus) -> StatusBadge {
        switch status {
        case .tracking:
            return .tracking
        case .paused, .onBreak:
            return .paused
        case .away, .locked, .asleep:
            return .away
        case .connecting, .offline, .choosingProject, .choosingWorkspace:
            return .waiting
        case .signedOut:
            return .signedOut
        case .signInExpired, .stopped:
            return .attention
        }
    }

    /// What VoiceOver reads for the icon.
    var accessibilityLabel: String {
        switch self {
        case .tracking:
            return "Dosi Tracker: tracking"
        case .paused:
            return "Dosi Tracker: paused"
        case .away:
            return "Dosi Tracker: not tracking"
        case .waiting:
            return "Dosi Tracker: waiting"
        case .signedOut:
            return "Dosi Tracker: signed out"
        case .attention:
            return "Dosi Tracker: needs attention"
        }
    }
}

enum StatusIcon {
    /// Points; the menu bar scales it for Retina.
    static let size = NSSize(width: 18, height: 18)

    static func image(_ badge: StatusBadge) -> NSImage {
        let image = NSImage(size: size, flipped: false) { rect in
            StatusIcon.draw(badge, in: rect)
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = badge.accessibilityLabel
        return image
    }

    private static func draw(_ badge: StatusBadge, in rect: NSRect) {
        NSColor.black.setStroke()
        NSColor.black.setFill()
        let centerX = rect.midX
        let centerY = rect.midY

        let ring = NSBezierPath(ovalIn: rect.insetBy(dx: 2, dy: 2))
        ring.lineWidth = 1.6
        if badge == .waiting {
            let dashes: [CGFloat] = [2.4, 1.8]
            ring.setLineDash(dashes, count: dashes.count, phase: 0)
        }
        ring.stroke()

        switch badge {
        case .tracking:
            NSBezierPath(ovalIn: NSRect(x: centerX - 3.2, y: centerY - 3.2, width: 6.4, height: 6.4)).fill()
        case .paused:
            NSBezierPath(rect: NSRect(x: centerX - 2.7, y: centerY - 3, width: 1.9, height: 6)).fill()
            NSBezierPath(rect: NSRect(x: centerX + 0.8, y: centerY - 3, width: 1.9, height: 6)).fill()
        case .away:
            let dot = NSBezierPath(ovalIn: NSRect(x: centerX - 2.6, y: centerY - 2.6, width: 5.2, height: 5.2))
            dot.lineWidth = 1.3
            dot.stroke()
        case .waiting:
            NSBezierPath(ovalIn: NSRect(x: centerX - 1.7, y: centerY - 1.7, width: 3.4, height: 3.4)).fill()
        case .signedOut:
            break
        case .attention:
            NSBezierPath(rect: NSRect(x: centerX - 0.9, y: centerY - 0.4, width: 1.8, height: 4.2)).fill()
            NSBezierPath(ovalIn: NSRect(x: centerX - 1, y: centerY - 3.4, width: 2, height: 2)).fill()
        }
    }
}
