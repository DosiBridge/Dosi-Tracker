import DosiTrackerCore
import SwiftUI

// The user's own consent on this Mac (port of the Windows agent's consent
// panel): one switch per kind of capture, what is never collected, and the
// standard choice or a revocation. Shown in Settings, and on the Timer tab
// while tracking waits for it (``TrackerStatus/consentRequired``). Nothing is
// captured that the user did not switch on here AND the project allows.

struct ConsentCard: View {
    @ObservedObject var model: AppModel
    /// Shown because tracking cannot start without it (the Timer tab).
    var asking = false

    var body: some View {
        let consent = model.localConsent
        let perms = consent.permissions
        return Card(asking ? "Choose what Dosi Tracker may record" : "Local Consent & Data Collection",
                    trailing: ConsentCard.badge(consent)) {
            if asking {
                Text("Your workspace's project decides what may be captured; on this Mac, you decide too. Nothing is recorded until you choose at least one of these. You can change your choice at any time in Settings.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Divider()
            }
            toggleRow("Keyboard Activity", "Record how many keys you press (never which ones, never what you type).",
                      perms.keyboard) { next, on in next.keyboard = on }
            Divider()
            toggleRow("Mouse Activity", "Record mouse clicks, scroll and movement counts.", perms.mouse) { next, on in
                next.mouse = on
            }
            Divider()
            toggleRow("Active Window & App", "Record which application and window title was active.",
                      perms.activeWindow) { next, on in next.activeWindow = on }
            Divider()
            toggleRow("Running Programs", "Record the names of the apps you have open.", perms.runningPrograms) { next, on in
                next.runningPrograms = on
            }
            Divider()
            toggleRow("Screen Captures", "Take one screenshot per time block of the screen you work on.",
                      perms.screenshot) { next, on in next.screenshot = on }
            if perms.screenshot {
                Divider()
                toggleRow("Blur Screenshots", "Blur screenshots beyond recognition before they leave this Mac.",
                          perms.blurScreenshots) { next, on in next.blurScreenshots = on }
            }
            Divider()
            toggleRow("Webcam Photos", "Take one webcam photo per time block (off unless you switch it on).",
                      perms.webcam) { next, on in next.webcam = on }
            Divider()
            HStack {
                if consent.isGranted {
                    Button(role: .destructive) {
                        model.revokeConsent()
                    } label: {
                        Text("Revoke All Consent")
                    }
                } else {
                    Button {
                        model.grantStandardConsent()
                    } label: {
                        Text("Grant Standard Consent")
                    }
                    .buttonStyle(.borderedProminent)
                    .help("Keyboard and mouse counts, the app in front, and blurred screenshots. No webcam, no running programs.")
                }
            }
            Text("Never collected: what you type, which keys you press, your clipboard, or the timing between your keystrokes. Capture runs only while you are tracking, and only what you switched on here and your project allows. Revoking stops capture at once.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    static func badge(_ consent: LocalConsent) -> String {
        switch consent {
        case .granted:
            return "Opted In"
        case .revoked:
            return "Revoked"
        case .absent:
            return "Not Consented"
        }
    }

    /// A switch for one capability: turning it on or off saves the whole
    /// choice (``AppModel/updateConsent(permissions:)``).
    private func toggleRow(_ title: String, _ description: String, _ isOn: Bool,
                           _ change: @escaping (inout ConsentedPermissions, Bool) -> Void) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                Text(description).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            Toggle("", isOn: Binding(
                get: { isOn },
                set: { on in
                    var next = model.localConsent.permissions
                    change(&next, on)
                    model.updateConsent(permissions: next)
                }
            ))
            .toggleStyle(.switch)
            .labelsHidden()
        }
    }
}
