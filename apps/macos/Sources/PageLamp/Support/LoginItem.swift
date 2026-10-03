// "Open PageLamp at login" (spec §3.5, M3; preview builds until it ships): the app itself as a
// login item through `SMAppService.mainApp`. The system keeps the registration (System Settings ▸
// General ▸ Login Items); PageLamp writes no launch agent or plist of its own. Off until the
// student turns it on. [verify] on the signed preview build: ad-hoc builds may not register.

import Foundation
import Observation
import ServiceManagement

@Observable @MainActor
final class LoginItem {
    enum State: Equatable {
        case off
        case on
        /// Registered, but the student must allow it in System Settings ▸ Login Items.
        case needsApproval
        /// The system can't find this app to register it (e.g. an unsigned copy).
        case unavailable
    }

    private(set) var state: State = .off
    /// The last change failed (said under the switch).
    private(set) var failed = false

    init() {
        refresh()
    }

    func refresh() {
        state = switch SMAppService.mainApp.status {
        case .enabled: .on
        case .requiresApproval: .needsApproval
        case .notFound: .unavailable
        default: .off
        }
    }

    func set(_ on: Bool) {
        do {
            if on {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            failed = false
        } catch {
            failed = true
        }
        refresh()
    }

    func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
