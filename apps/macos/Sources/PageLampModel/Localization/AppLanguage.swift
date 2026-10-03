// The language and appearance settings (Settings ▸ General, spec §3.5).

import Foundation

/// The app's display language. View content switches live; menus and system dialogs follow
/// `AppleLanguages`, which only changes after the app reopens (spec §7.3).
public enum AppLanguage: String, CaseIterable, Codable, Sendable {
    case system
    case english = "en"
    case simplifiedChinese = "zh-Hans"

    /// The localizations the app ships (CFBundleLocalizations).
    public static let localizations = ["en", "zh-Hans"]

    /// The shipped localization this setting resolves to, given the system's preferred languages.
    public func localization(preferredLanguages: [String]) -> String {
        switch self {
        case .english: "en"
        case .simplifiedChinese: "zh-Hans"
        case .system:
            Bundle.preferredLocalizations(
                from: Self.localizations,
                forPreferences: preferredLanguages
            ).first ?? "en"
        }
    }

    /// Locale for formatting: the resolved language with the system's region, so dates, times
    /// and numbers read naturally ("9月26日 星期五", 24-hour where the region uses it).
    public func locale(preferredLanguages: [String], region: Locale.Region?) -> Locale {
        var components = Locale.Components(
            languageCode: nil,
            script: nil,
            languageRegion: region
        )
        components.languageComponents = Locale.Language.Components(
            identifier: localization(preferredLanguages: preferredLanguages)
        )
        return Locale(components: components)
    }

    /// The `AppleLanguages` override that makes menus follow this setting after a relaunch
    /// (nil = follow the system).
    public var appleLanguages: [String]? {
        switch self {
        case .system: nil
        case .english: ["en"]
        case .simplifiedChinese: ["zh-Hans"]
        }
    }
}

/// Settings ▸ General ▸ Appearance.
public enum AppAppearance: String, CaseIterable, Codable, Sendable {
    case system
    case light
    case dark
}

/// Where the model keeps the student's settings. UserDefaults in the app; in memory in tests
/// (so tests never write the user's preferences).
public protocol SettingsStore: AnyObject, Sendable {
    var language: AppLanguage { get set }
    var appearance: AppAppearance { get set }
    /// The app-domain `AppleLanguages` override (Reopen Now); nil removes it.
    var appleLanguages: [String]? { get set }
    /// Reminders handed to the system's notification center that aren't marked shown yet, with
    /// when they fire (`ReminderDelivery`): what the system holds, not policy.
    var handedOverReminders: [String: Date] { get set }
}

/// `UserDefaults.standard` (the app's own domain).
public final class UserDefaultsSettingsStore: SettingsStore, @unchecked Sendable {
    // UserDefaults is thread-safe; the class holds no other state.
    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public var language: AppLanguage {
        get { defaults.string(forKey: "language").flatMap(AppLanguage.init(rawValue:)) ?? .system }
        set { defaults.set(newValue.rawValue, forKey: "language") }
    }

    public var appearance: AppAppearance {
        get { defaults.string(forKey: "appearance").flatMap(AppAppearance.init(rawValue:)) ?? .system }
        set { defaults.set(newValue.rawValue, forKey: "appearance") }
    }

    public var appleLanguages: [String]? {
        get { defaults.stringArray(forKey: "AppleLanguages") }
        set {
            if let newValue {
                defaults.set(newValue, forKey: "AppleLanguages")
            } else {
                defaults.removeObject(forKey: "AppleLanguages")
            }
        }
    }
}

extension UserDefaultsSettingsStore {
    public var handedOverReminders: [String: Date] {
        get {
            let stored = defaults.dictionary(forKey: "handedOverReminders") as? [String: Double] ?? [:]
            return stored.mapValues { Date(timeIntervalSince1970: $0) }
        }
        set {
            defaults.set(newValue.mapValues(\.timeIntervalSince1970), forKey: "handedOverReminders")
        }
    }
}

/// Settings that live only as long as the process (tests, snapshots).
public final class InMemorySettingsStore: SettingsStore, @unchecked Sendable {
    // Only touched from the main actor in practice; the lock keeps the Sendable promise honest.
    private let lock = NSLock()
    private var _language: AppLanguage
    private var _appearance: AppAppearance
    private var _appleLanguages: [String]?
    private var _handedOverReminders: [String: Date] = [:]

    public init(language: AppLanguage = .system, appearance: AppAppearance = .system) {
        _language = language
        _appearance = appearance
    }

    public var language: AppLanguage {
        get { lock.withLock { _language } }
        set { lock.withLock { _language = newValue } }
    }

    public var appearance: AppAppearance {
        get { lock.withLock { _appearance } }
        set { lock.withLock { _appearance = newValue } }
    }

    public var appleLanguages: [String]? {
        get { lock.withLock { _appleLanguages } }
        set { lock.withLock { _appleLanguages = newValue } }
    }

    public var handedOverReminders: [String: Date] {
        get { lock.withLock { _handedOverReminders } }
        set { lock.withLock { _handedOverReminders = newValue } }
    }
}
