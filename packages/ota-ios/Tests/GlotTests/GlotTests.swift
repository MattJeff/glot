import Foundation
import Testing

@testable import Glot

/// Fake CDN: URLProtocol that answers from `Stub.handler` and records every request.
final class Stub: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: (URLRequest) -> (Int, [String: String], Data)? = { _ in nil }
    nonisolated(unsafe) static var requests: [URLRequest] = []

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}
    override func startLoading() {
        Stub.requests.append(request)
        guard let (code, headers, body) = Stub.handler(request) else {
            client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
            return
        }
        let res = HTTPURLResponse(url: request.url!, statusCode: code, httpVersion: nil, headerFields: headers)!
        client?.urlProtocol(self, didReceive: res, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }

    static var session: URLSession {
        let c = URLSessionConfiguration.ephemeral
        c.protocolClasses = [Stub.self]
        return URLSession(configuration: c)
    }
}

let locales = #"{"source":"en","locales":[{"code":"en","ready":true},{"code":"fr","ready":true},{"code":"pl","ready":true},{"code":"de","ready":false}]}"#
let fr = #"{"home":{"title":"Bonjour"},"items":"{count, plural, one {%lld élément} other {%lld éléments}}","bad":"%@ x"}"#
let pl = #"{"home":{"title":"Cześć"},"items":"{count, plural, one {%lld element} few {%lld elementy} many {%lld elementów} other {%lld elementu}}"}"#

/// A CDN serving `files`, with ETags and 304s.
func cdn(_ files: [String: String]) -> (URLRequest) -> (Int, [String: String], Data)? {
    { req in
        guard let name = req.url?.lastPathComponent, let body = files[name] else { return (404, [:], Data()) }
        let tag = "\"\(body.hashValue)\""
        if req.value(forHTTPHeaderField: "If-None-Match") == tag { return (304, ["ETag": tag], Data()) }
        return (200, ["ETag": tag, "Content-Type": "application/json"], Data(body.utf8))
    }
}

func tempDir() -> URL {
    let d = FileManager.default.temporaryDirectory.appendingPathComponent("glot-test-\(UUID().uuidString)")
    try! FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
    return d
}

/// A bundle with an English Localizable.strings, like the one Xcode compiles from Localizable.xcstrings.
func embeddedBundle() -> Bundle {
    let lproj = tempDir().appendingPathComponent("App.bundle/en.lproj")
    try! FileManager.default.createDirectory(at: lproj, withIntermediateDirectories: true)
    try! "\"shipped\" = \"Shipped in the app\";\n\"hello\" = \"Hello %@\";".write(
        to: lproj.appendingPathComponent("Localizable.strings"), atomically: true, encoding: .utf8)
    return Bundle(url: lproj.deletingLastPathComponent())!
}

func configure(_ cache: URL, locale: String? = "fr", bundle: Bundle = .main) {
    Glot.store.configure("demo", URL(string: "https://glot.test")!, locale, bundle, nil, cache, Stub.session)
}

@Suite(.serialized) struct GlotTests {
    @Test func pluralRulesMatchGlotValidator() throws {
        let url = URL(fileURLWithPath: #filePath).absoluteURL.deletingLastPathComponent()
            .appendingPathComponent("../../../ota-fixtures/plural-cases.json").standardizedFileURL
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
        let numbers = fixture["numbers"] as! [String]
        let cases = fixture["categories"] as! [String: [String]]
        var wrong: [String] = []
        for (locale, expected) in cases {
            for (n, want) in zip(numbers, expected) where Plural.category(locale, n) != want {
                wrong.append("\(locale) \(n): \(Plural.category(locale, n)) != \(want)")
            }
        }
        #expect(cases.count > 150)  // every language ICU4X (glot's validator) has data for
        #expect(wrong.isEmpty, "\(wrong.prefix(20))")
    }

    @Test func icuPluralsSelectsAndSubstitutions() {
        let ru = "{count, plural, one {%lld файл} few {%lld файла} many {%lld файлов} other {%lld файла}}"
        #expect(ICU.expand(ru, locale: "ru", numbers: ["21"], selects: [:]) == "%lld файл")
        #expect(ICU.expand(ru, locale: "ru", numbers: ["3"], selects: [:]) == "%lld файла")
        #expect(ICU.expand(ru, locale: "ru", numbers: ["11"], selects: [:]) == "%lld файлов")
        let zero = "{count, plural, =0 {Empty} one {# item} other {# items}}"
        #expect(ICU.expand(zero, locale: "en", numbers: ["0"], selects: [:]) == "Empty")
        #expect(ICU.expand(zero, locale: "en", numbers: ["7"], selects: [:]) == "7 items")
        let device = "{device, select, iphone {Tap} mac {Click} other {Press}}"
        #expect(ICU.expand(device, locale: "en", numbers: [], selects: ["device": "mac"]) == "Click")
        #expect(ICU.expand(device, locale: "en", numbers: [], selects: [:]) == "Press")
        // String Catalog substitutions: two plurals, each takes its own argument; `%arg` → `%lld`.
        let subs = "{a, plural, one {%arg file} other {%arg files}} in {b, plural, one {%arg folder} other {%arg folders}}"
        let out = ICU.expand(subs, locale: "en", numbers: ["1", "4"], selects: [:])
        #expect(out == "%lld file in %lld folders")
        #expect(Printf.format(out, [1, 4], locale: Locale(identifier: "en")) == "1 file in 4 folders")
        #expect(ICU.expand("Literal {name} and {", locale: "en", numbers: [], selects: [:]) == "Literal {name} and {")
    }

    @Test func printfNeverCrashes() {
        let en = Locale(identifier: "en")
        #expect(Printf.format("%@ has %lld", ["Ana", 3], locale: en) == "Ana has 3")
        #expect(Printf.format("%2$@ %1$lld", [3, "x"], locale: en) == "x 3")
        #expect(Printf.format("%@", [3], locale: en) == nil)  // would crash String(format:)
        #expect(Printf.format("%lld %lld", [3], locale: en) == nil)  // missing argument
        #expect(Printf.format("%s", ["x"], locale: en) == nil)
        #expect(Printf.format("100%% %.1f", [2.5], locale: en) == "100% 2.5")
    }

    @Test func downloadsCachesAndRevalidates() async {
        let cache = tempDir()
        Stub.requests = []
        Stub.handler = cdn(["locales.json": locales, "fr.json": fr])
        configure(cache)
        #expect(Glot.string("home.title", default: "Hello") == "Hello")  // nothing cached yet

        var notified = false
        let token = NotificationCenter.default.addObserver(forName: .glotDidUpdate, object: nil, queue: nil) { _ in
            notified = true
        }
        defer { NotificationCenter.default.removeObserver(token) }
        #expect(await Glot.refresh())
        #expect(Glot.locale == "fr")
        #expect(Glot.locales == ["en", "fr", "pl"])  // de not ready
        #expect(Glot.string("home.title", default: "Hello") == "Bonjour")
        #expect(Glot.string("items", default: "%lld items", 1) == "1 élément")
        #expect(Glot.string("items", default: "%lld items", 5) == "5 éléments")
        try? await Task.sleep(nanoseconds: 50_000_000)
        #expect(notified)

        // Second check: ETags sent, 304s, nothing changes. Only If-None-Match (and a generic User-Agent) is ever sent.
        Stub.requests = []
        #expect(await Glot.refresh() == false)
        #expect(Stub.requests.count == 2)
        #expect(Stub.requests.allSatisfy { $0.value(forHTTPHeaderField: "If-None-Match") != nil })
        #expect(Stub.requests.allSatisfy { Set(($0.allHTTPHeaderFields ?? [:]).keys).isSubset(of: ["If-None-Match", "User-Agent"]) })
        #expect(Stub.requests.map { $0.url!.path } == ["/cdn/demo/locales.json", "/cdn/demo/fr.json"])

        // Restart offline: yesterday's catalog is used right away, refreshing fails quietly.
        Stub.handler = { _ in nil }
        configure(cache)
        #expect(Glot.string("home.title", default: "Hello") == "Bonjour")
        #expect(await Glot.refresh() == false)
        #expect(Glot.string("home.title", default: "Hello") == "Bonjour")
    }

    @Test func newLanguageWithoutAnUpdate() async {
        Stub.handler = cdn(["locales.json": locales, "fr.json": fr, "pl.json": pl])
        configure(tempDir())
        await Glot.refresh()
        #expect(await Glot.setLocale("pl-PL"))
        #expect(Glot.locale == "pl")
        #expect(Glot.string("items", default: "%lld items", 3) == "3 elementy")
        #expect(Glot.string("items", default: "%lld items", 5) == "5 elementów")
        #expect(Store.pick(nil, ["en", "fr"], .main) != "de")  // not ready: never picked from preferences
        #expect(Store.pick("fr-CA", ["en", "fr"], .main) == "fr")
    }

    @Test func fallsBackToBundleThenDefault() async {
        let bundle = embeddedBundle()
        for broken in [cdn([:]), { _ in (500, [:], Data()) }, { _ in nil }, cdn(["locales.json": "{", "fr.json": "nope"])]
            as [(URLRequest) -> (Int, [String: String], Data)?]
        {
            Stub.handler = broken
            configure(tempDir(), bundle: bundle)
            #expect(await Glot.refresh() == false)
            #expect(Glot.string("shipped", default: "x") == "Shipped in the app")
            #expect(Glot.string("hello", default: "x", "Ana") == "Hello Ana")
            #expect(Glot.string("missing", default: "Default %lld", 2) == "Default 2")
            #expect(Glot.string("missing") == "missing")
        }
    }

    @Test func badTranslationPlaceholderFallsBack() async {
        Stub.handler = cdn(["locales.json": locales, "fr.json": fr])
        configure(tempDir())
        await Glot.refresh()
        // "bad" is "%@ x" in the CDN; fed an Int it would crash String(format:): the default is used.
        #expect(Glot.string("bad", default: "%lld y", 4) == "4 y")
        #expect(Glot.string("bad", default: "%lld y") == "%lld y")  // no argument at all: raw default
    }
}
