import Foundation

public extension Notification.Name {
    /// Posted on the main thread when new strings (or a new language) arrived from the CDN.
    static let glotDidUpdate = Notification.Name("GlotDidUpdate")
}

/// Over-the-air strings from glot's CDN, on top of the String Catalog compiled into your app.
///
/// Lookup order: the downloaded catalog for `Glot.locale`, then your bundle (`.xcstrings` / `.strings` /
/// `.stringsdict`), then `default`. Nothing about the user is sent: two GETs (`locales.json`, `{locale}.json`)
/// with `If-None-Match`, no cookies, an ephemeral session. A network or CDN failure only means the last
/// downloaded catalog (or the bundle) keeps being used.
public enum Glot {
    /// - Parameters:
    ///   - project: the project slug (`/cdn/{project}/…`). The project's CDN must be public.
    ///   - cdn: your glot server, e.g. `https://api.example.com`.
    ///   - locale: force a language; default: the first of `Locale.preferredLanguages` glot has ready.
    public static func configure(
        project: String, cdn: URL, locale: String? = nil, bundle: Bundle = .main, table: String? = nil,
        cacheDirectory: URL? = nil, session: URLSession? = nil
    ) {
        store.configure(project, cdn, locale, bundle, table, cacheDirectory, session)
        Task.detached { await store.refresh() }
    }

    /// The translation of `key`, formatted with `args` (`%@`, `%lld`, `%1$@`…; plural variations pick
    /// their form from the numeric arguments, in order).
    public static func string(_ key: String, default value: String? = nil, _ args: CVarArg...) -> String {
        store.string(key, value, args)
    }

    public static func string(_ key: String, default value: String? = nil, arguments: [CVarArg]) -> String {
        store.string(key, value, arguments)
    }

    /// Re-checks the CDN (call it from `scenePhase == .active`, for instance). True when strings changed.
    @discardableResult public static func refresh() async -> Bool { await store.refresh() }

    /// Switch language (nil: back to the system preference), downloading it if needed.
    @discardableResult public static func setLocale(_ code: String?) async -> Bool {
        store.setPreferred(code)
        return await store.refresh()
    }

    /// The language strings are served in.
    public static var locale: String { store.read { $0.locale } }
    /// Languages glot has ready (from `locales.json`), source language first: offer them in a picker,
    /// including languages your build does not embed.
    public static var locales: [String] { store.read { $0.locales } }

    static let store = Store()
}

final class Store: @unchecked Sendable {
    struct State {
        var project = "", cdn = URL(fileURLWithPath: "/"), preferred: String?
        var bundle = Bundle.main, table: String?, dir = URL(fileURLWithPath: NSTemporaryDirectory())
        var session = URLSession(configuration: .ephemeral)
        var locale = "en", locales: [String] = [], messages: [String: String] = [:]
    }

    private var state = State()
    private let lock = NSLock()

    func read<T>(_ f: (State) -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return f(state)
    }

    private func write(_ f: (inout State) -> Void) {
        lock.lock()
        f(&state)
        lock.unlock()
    }

    func configure(
        _ project: String, _ cdn: URL, _ locale: String?, _ bundle: Bundle, _ table: String?, _ dir: URL?,
        _ session: URLSession?
    ) {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let config = URLSessionConfiguration.ephemeral
        config.httpShouldSetCookies = false
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        var s = State()
        s.project = project
        s.cdn = cdn
        s.preferred = locale
        s.bundle = bundle
        s.table = table
        s.dir = (dir ?? caches).appendingPathComponent("glot").appendingPathComponent(safe(project) ?? "_")
        s.session = session ?? URLSession(configuration: config)
        // Last downloaded catalog first: the app starts with yesterday's fixes even offline.
        s.locales = (cached(s.dir, "locales.json")).flatMap(Self.parseLocales) ?? []
        s.locale = Self.pick(s.preferred, s.locales, bundle)
        s.messages = cached(s.dir, "\(s.locale).json").flatMap(Self.parseCatalog) ?? [:]
        write { $0 = s }
    }

    func setPreferred(_ code: String?) { write { $0.preferred = code } }

    func string(_ key: String, _ value: String?, _ args: [CVarArg]) -> String {
        let (locale, ota, bundle, table) = read { ($0.locale, $0.messages[key], $0.bundle, $0.table) }
        let numbers = args.compactMap(Printf.number)
        let selects = ["device": Self.device]
        let loc = Locale(identifier: locale)
        if let ota, let s = Printf.format(ICU.expand(ota, locale: locale, numbers: numbers, selects: selects), args, locale: loc) {
            return s
        }
        let missing = "\u{0}glot-missing"
        let embedded = bundle.localizedString(forKey: key, value: missing, table: table)
        if embedded != missing, let s = Printf.format(embedded, args, locale: loc) { return s }
        let fallback = value ?? key
        return Printf.format(ICU.expand(fallback, locale: locale, numbers: numbers, selects: selects), args, locale: loc)
            ?? fallback
    }

    @discardableResult func refresh() async -> Bool {
        let s = read { $0 }
        guard !s.project.isEmpty, let project = safe(s.project) else { return false }
        let locales = await get(s, project, "locales.json").flatMap { Self.parseLocales($0.data) } ?? s.locales
        let locale = Self.pick(s.preferred, locales, s.bundle)
        let got = await get(s, project, "\(locale).json")
        let messages = got.flatMap { $0.fresh || locale != s.locale ? Self.parseCatalog($0.data) : nil }
        var changed = false
        write { st in
            guard st.project == s.project else { return }  // reconfigured meanwhile
            st.locales = locales
            if let messages {
                changed = messages != st.messages || locale != st.locale
                st.messages = messages
                st.locale = locale
            }  // else: new language unreachable, keep the current one and its strings rather than mix
        }
        if changed {
            DispatchQueue.main.async { NotificationCenter.default.post(name: .glotDidUpdate, object: nil) }
        }
        return changed
    }

    /// GET with `If-None-Match`; 304 → the cached body. Nil on any failure.
    private func get(_ s: State, _ project: String, _ file: String) async -> (data: Data, fresh: Bool)? {
        guard safe(file.replacingOccurrences(of: ".json", with: "")) != nil else { return nil }
        let url = s.cdn.appendingPathComponent("cdn").appendingPathComponent(project).appendingPathComponent(file)
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
        req.httpShouldHandleCookies = false
        req.setValue("glot-ota", forHTTPHeaderField: "User-Agent")  // not the app name / OS version
        let etag = s.dir.appendingPathComponent(file + ".etag")
        let cachedBody = cached(s.dir, file)
        if cachedBody != nil, let tag = try? String(contentsOf: etag, encoding: .utf8) {
            req.setValue(tag, forHTTPHeaderField: "If-None-Match")
        }
        guard let (data, res) = try? await s.session.data(for: req), let http = res as? HTTPURLResponse else {
            return nil
        }
        if http.statusCode == 304 { return cachedBody.map { ($0, false) } }
        guard http.statusCode == 200 else { return nil }
        try? FileManager.default.createDirectory(at: s.dir, withIntermediateDirectories: true)
        try? data.write(to: s.dir.appendingPathComponent(file), options: .atomic)
        if let tag = http.value(forHTTPHeaderField: "ETag") {
            try? tag.write(to: etag, atomically: true, encoding: .utf8)
        }
        return (data, true)
    }

    private func cached(_ dir: URL, _ file: String) -> Data? {
        try? Data(contentsOf: dir.appendingPathComponent(file))
    }

    // MARK: parsing (never throws: a bad body is ignored)

    static func parseCatalog(_ data: Data) -> [String: String]? {
        guard let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        var out: [String: String] = [:]
        func walk(_ prefix: String, _ v: Any) {
            if let s = v as? String {
                out[prefix] = s
            } else if let o = v as? [String: Any] {
                for (k, sub) in o { walk(prefix.isEmpty ? k : "\(prefix).\(k)", sub) }
            }
        }
        walk("", root)
        return out
    }

    static func parseLocales(_ data: Data) -> [String]? {
        guard let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let list = root["locales"] as? [[String: Any]]
        else { return nil }
        return list.compactMap { e in
            guard let code = e["code"] as? String, safe(code) != nil, e["ready"] as? Bool ?? true else { return nil }
            return code
        }
    }

    /// Explicit choice, else the first preferred language glot has ready (exact, then same language),
    /// else the bundle's own language.
    static func pick(_ preferred: String?, _ available: [String], _ bundle: Bundle) -> String {
        let wanted = preferred.map { [$0] } ?? Locale.preferredLanguages
        let norm = { (s: String) in s.replacingOccurrences(of: "_", with: "-").lowercased() }
        let base = { (s: String) in String(norm(s).split(separator: "-").first ?? "") }
        for w in wanted {
            if let hit = available.first(where: { norm($0) == norm(w) }) { return hit }
            if let hit = available.first(where: { base($0) == base(w) }) { return hit }
        }
        if let preferred, safe(preferred) != nil { return preferred }
        return bundle.preferredLocalizations.first ?? available.first ?? "en"
    }

    static let device: String = {
        #if os(macOS)
            return "mac"
        #elseif os(watchOS)
            return "applewatch"
        #elseif os(tvOS)
            return "appletv"
        #elseif os(visionOS)
            return "applevision"
        #else
            return ProcessInfo.processInfo.isiOSAppOnMac ? "mac" : (UIKitIdiom.isPad ? "ipad" : "iphone")
        #endif
    }()
}

/// Slugs and locale codes go into a URL path and a file name: letters, digits, `-`, `_` only.
func safe(_ s: String) -> String? {
    !s.isEmpty && s.count <= 64 && s.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") })
        ? s : nil
}

#if canImport(UIKit) && !os(watchOS)
    import UIKit
    enum UIKitIdiom { static var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad } }
#else
    enum UIKitIdiom { static let isPad = false }
#endif
