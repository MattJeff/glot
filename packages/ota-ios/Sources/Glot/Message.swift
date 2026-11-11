import Foundation

// MARK: CLDR cardinal plural rules (data: PluralData.swift, same CLDR rules as glot's validator)

enum Plural {
    static let rules: [String: [String: String]] =
        (try? JSONDecoder().decode([String: [String: String]].self, from: Data(pluralData.utf8))) ?? [:]
    static let order = ["zero", "one", "two", "few", "many"]

    /// CLDR category of `number` (a decimal literal: "1", "1.50") in `locale` ("pt-PT", "fr_CA", "ru").
    static func category(_ locale: String, _ number: String) -> String {
        let tag = locale.replacingOccurrences(of: "_", with: "-")
        let base = String(tag.split(separator: "-").first ?? "")
        guard let r = rules[tag] ?? rules[base] else { return "other" }
        let ops = Operands(number)
        return order.first { r[$0].map { matches($0, ops) } ?? false } ?? "other"
    }

    static func matches(_ cond: String, _ o: Operands) -> Bool {
        cond.components(separatedBy: " or ").contains { and in
            and.components(separatedBy: " and ").allSatisfy { relation($0, o) }
        }
    }

    private static func relation(_ rel: String, _ o: Operands) -> Bool {
        let p = rel.split(separator: " ").map(String.init)
        guard p.count == 3 || p.count == 5 else { return false }
        var x = o.value(p[0])
        if p.count == 5 { x = fmod(x, Double(p[2]) ?? 1) }
        let hit = p[p.count - 1].split(separator: ",").contains { range in
            let b = range.components(separatedBy: "..").compactMap(Double.init)
            guard let lo = b.first, x == x.rounded(.towardZero) else { return false }
            return x >= lo && x <= (b.last ?? lo)
        }
        return p[p.count - 2] == "=" ? hit : !hit
    }
}

struct Operands {
    var n = 0.0, i = 0.0, v = 0.0, w = 0.0, f = 0.0, t = 0.0

    init(_ s: String) {
        let s = s.hasPrefix("-") ? String(s.dropFirst()) : s
        let parts = s.split(separator: ".", maxSplits: 1).map(String.init)
        n = Double(s) ?? 0
        i = Double(parts.first ?? "0") ?? 0
        let frac = parts.count > 1 ? parts[1] : ""
        let trimmed = String(frac.reversed().drop { $0 == "0" }.reversed())
        v = Double(frac.count)
        w = Double(trimmed.count)
        f = Double(frac) ?? 0
        t = Double(trimmed) ?? 0
    }

    func value(_ op: String) -> Double {
        switch op {
        case "n": n
        case "i": i
        case "v": v
        case "w": w
        case "f": f
        case "t": t
        default: 0  // e, c: compact exponent, never used for plain numbers
        }
    }
}

// MARK: ICU plural / select (what glot stores for String Catalog variations and substitutions)

enum ICU {
    /// Expands `{count, plural, one {…} other {…}}` and `{device, select, …}` blocks. Plural blocks take the
    /// numeric arguments in order (`%#@a@ … %#@b@` → arg 1, arg 2); `%arg` in a branch becomes `%lld`.
    static func expand(_ msg: String, locale: String, numbers: [String], selects: [String: String]) -> String {
        var next = 0
        return expand(Array(msg), locale, numbers, selects, &next)
    }

    private static func expand(
        _ s: [Character], _ locale: String, _ numbers: [String], _ selects: [String: String], _ next: inout Int
    ) -> String {
        var out = ""
        var i = 0
        while i < s.count {
            guard s[i] == "{", let end = close(s, i), let choice = choice(Array(s[(i + 1)..<end])) else {
                out.append(s[i])
                i += 1
                continue
            }
            out += pick(choice, locale, numbers, selects, &next)
            i = end + 1
        }
        return out
    }

    private static func pick(
        _ c: (variable: String, kind: String, forms: [(String, [Character])]), _ locale: String,
        _ numbers: [String], _ selects: [String: String], _ next: inout Int
    ) -> String {
        let form: (String) -> [Character]? = { k in c.forms.first { $0.0 == k }?.1 }
        guard c.kind != "select" else {
            let body = form(selects[c.variable] ?? "other") ?? form("other") ?? c.forms.first?.1 ?? []
            return expand(body, locale, numbers, selects, &next)
        }
        let number = next < numbers.count ? numbers[next] : "0"
        next += 1
        // ponytail: selectordinal has no ordinal data here and takes `other` (exact `=N` still matches).
        let category = c.kind == "plural" ? Plural.category(locale, number) : "other"
        let exact = c.forms.first { $0.0.hasPrefix("=") && Double($0.0.dropFirst()) == Double(number) }?.1
        var body = exact ?? form(category) ?? form("other") ?? c.forms.first?.1 ?? []
        var depth = 0
        for (k, ch) in body.enumerated() {  // `#` at this level is the number
            if ch == "{" { depth += 1 } else if ch == "}" { depth -= 1 }
            if ch == "#" && depth == 0 { body[k] = "\u{0}" }
        }
        return expand(body, locale, numbers, selects, &next)
            .replacingOccurrences(of: "\u{0}", with: number)
            .replacingOccurrences(of: "%arg", with: "%lld")
    }

    private static func close(_ s: [Character], _ open: Int) -> Int? {
        var depth = 0
        for k in open..<s.count {
            if s[k] == "{" { depth += 1 } else if s[k] == "}" {
                depth -= 1
                if depth == 0 { return k }
            }
        }
        return nil
    }

    /// `var, kind, sel {body} sel {body}` → parts; nil for anything else (a literal brace, `{name}`).
    private static func choice(_ s: [Character]) -> (variable: String, kind: String, forms: [(String, [Character])])? {
        let head = s.split(separator: ",", maxSplits: 2, omittingEmptySubsequences: false)
        guard head.count == 3 else { return nil }
        let kind = String(head[1]).trimmingCharacters(in: .whitespaces)
        guard ["plural", "select", "selectordinal"].contains(kind) else { return nil }
        var rest = Array(head[2])
        var forms: [(String, [Character])] = []
        while let open = rest.firstIndex(of: "{") {
            let key = String(rest[..<open]).trimmingCharacters(in: .whitespacesAndNewlines)
                .split(separator: " ").last.map(String.init) ?? ""  // drops `offset:n`
            guard let end = close(rest, open) else { return nil }
            forms.append((key, Array(rest[(open + 1)..<end])))
            rest = Array(rest[(end + 1)...])
        }
        return forms.isEmpty ? nil : (String(head[0]).trimmingCharacters(in: .whitespaces), kind, forms)
    }
}

// MARK: printf formatting that can never crash

enum Printf {
    private static let spec = try! NSRegularExpression(
        pattern: "%(?:(\\d+)\\$)?[-+ #0']*\\d*(?:\\.\\d+)?(hh|h|ll|l|q|z|t|j|L)?([@dDiuUxXoOcCfFeEgGaAsSpn%])")

    /// `String(format:)` only when every specifier matches the type of its argument (a `%@` fed an Int
    /// crashes the app); nil otherwise, so the caller falls back.
    static func format(_ fmt: String, _ args: [CVarArg], locale: Locale) -> String? {
        var seq = 0
        let ns = fmt as NSString
        for m in spec.matches(in: fmt, range: NSRange(location: 0, length: ns.length)) {
            let conv = ns.substring(with: m.range(at: 3))
            if conv == "%" { continue }
            let index: Int
            if m.range(at: 1).location != NSNotFound {
                index = (Int(ns.substring(with: m.range(at: 1))) ?? 0) - 1
            } else {
                index = seq
                seq += 1
            }
            guard index >= 0, index < args.count, accepts(conv, args[index]) else { return nil }
        }
        return String(format: fmt, locale: locale, arguments: args)
    }

    private static func accepts(_ conv: String, _ arg: CVarArg) -> Bool {
        switch conv {
        case "@": return arg is String || Swift.type(of: arg) is AnyObject.Type  // not `is NSObject`: an Int bridges
        case "d", "D", "i", "u", "U", "x", "X", "o", "O", "c", "C":
            return arg is Int || arg is Int8 || arg is Int16 || arg is Int32 || arg is Int64
                || arg is UInt || arg is UInt8 || arg is UInt16 || arg is UInt32 || arg is UInt64
        case "f", "F", "e", "E", "g", "G", "a", "A": return arg is Double || arg is Float
        default: return false  // %s %S %p %n: raw pointers, never from a translation
        }
    }

    /// Decimal literal of a numeric argument, for plural selection.
    static func number(_ arg: CVarArg) -> String? {
        switch arg {
        case let v as Int: String(v)
        case let v as Int32: String(v)
        case let v as Int64: String(v)
        case let v as UInt: String(v)
        case let v as UInt32: String(v)
        case let v as UInt64: String(v)
        case let v as Double: v == v.rounded() && abs(v) < 1e15 ? String(Int64(v)) : String(v)
        case let v as Float: Double(v) == Double(v).rounded() ? String(Int64(v)) : String(v)
        default: nil
        }
    }
}
