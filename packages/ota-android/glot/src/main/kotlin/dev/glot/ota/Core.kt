package dev.glot.ota

import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.Locale
import java.util.concurrent.Executors

// Pure JVM part of the SDK (no android.*): plural rules, ICU expansion, formatting, CDN + disk cache.
// Unit-tested on the JVM; Glot.kt wires it to Android Resources.

/** CLDR cardinal plural rules (data: PluralData.kt, the CLDR rules glot's validator uses). */
internal object Plural {
    private val order = listOf("zero", "one", "two", "few", "many")

    @Suppress("UNCHECKED_CAST")
    private val rules: Map<String, Map<String, String>> by lazy {
        (Json.parse(PLURAL_DATA) as? Map<String, Map<String, String>>) ?: emptyMap()
    }

    /** CLDR category of [number] (a decimal literal: "1", "1.50") in [locale] ("pt-PT", "fr_CA", "ru"). */
    fun category(locale: String, number: String): String {
        val tag = locale.replace('_', '-')
        val r = rules[tag] ?: rules[tag.substringBefore('-')] ?: return "other"
        val ops = Operands(number)
        return order.firstOrNull { cat -> r[cat]?.let { matches(it, ops) } == true } ?: "other"
    }

    fun matches(cond: String, o: Operands): Boolean =
        cond.split(" or ").any { and -> and.split(" and ").all { relation(it, o) } }

    private fun relation(rel: String, o: Operands): Boolean {
        val p = rel.split(' ')
        if (p.size != 3 && p.size != 5) return false
        var x = o.value(p[0])
        if (p.size == 5) x %= p[2].toDoubleOrNull() ?: 1.0
        val hit = p.last().split(',').any { range ->
            val b = range.split("..").mapNotNull { it.toDoubleOrNull() }
            b.isNotEmpty() && x == Math.floor(x) && x >= b.first() && x <= b.last()
        }
        return if (p[p.size - 2] == "=") hit else !hit
    }
}

internal class Operands(s: String) {
    private val n: Double
    private val i: Double
    private val v: Double
    private val w: Double
    private val f: Double
    private val t: Double

    init {
        val abs = s.removePrefix("-")
        val frac = abs.substringAfter('.', "")
        val trimmed = frac.trimEnd('0')
        n = abs.toDoubleOrNull() ?: 0.0
        i = abs.substringBefore('.').toDoubleOrNull() ?: 0.0
        v = frac.length.toDouble()
        w = trimmed.length.toDouble()
        f = frac.toDoubleOrNull() ?: 0.0
        t = trimmed.toDoubleOrNull() ?: 0.0
    }

    fun value(op: String): Double = when (op) {
        "n" -> n
        "i" -> i
        "v" -> v
        "w" -> w
        "f" -> f
        "t" -> t
        else -> 0.0 // e, c: compact exponent, never set for plain numbers
    }
}

/** `{count, plural, one {…} other {…}}` / `{x, select, …}` — what glot stores for `<plurals>`. */
internal object Icu {
    fun expand(msg: String, locale: String, numbers: List<String>, selects: Map<String, String> = emptyMap()): String {
        var next = 0
        fun run(s: String): String {
            val out = StringBuilder()
            var i = 0
            while (i < s.length) {
                val end = if (s[i] == '{') close(s, i) else -1
                val c = if (end > 0) choice(s.substring(i + 1, end)) else null
                if (c == null) {
                    out.append(s[i++])
                    continue
                }
                val (variable, kind, forms) = c
                fun form(k: String) = forms.firstOrNull { it.first == k }?.second
                out.append(
                    if (kind == "select") {
                        run(form(selects[variable] ?: "other") ?: form("other") ?: forms.first().second)
                    } else {
                        val number = numbers.getOrElse(next) { "0" }
                        next++
                        // ponytail: selectordinal has no ordinal data here and takes `other` (`=N` still matches).
                        val cat = if (kind == "plural") Plural.category(locale, number) else "other"
                        val exact = forms.firstOrNull {
                            it.first.startsWith("=") && it.first.drop(1).toDoubleOrNull() == number.toDoubleOrNull()
                        }?.second
                        val body = exact ?: form(cat) ?: form("other") ?: forms.first().second
                        run(hashes(body)).replace('\u0000'.toString(), number)
                    },
                )
                i = end + 1
            }
            return out.toString()
        }
        return run(msg)
    }

    /** `#` at this nesting level → placeholder for the number. */
    private fun hashes(body: String): String {
        var depth = 0
        return body.map { ch ->
            if (ch == '{') depth++ else if (ch == '}') depth--
            if (ch == '#' && depth == 0) '\u0000' else ch
        }.joinToString("")
    }

    private fun close(s: String, open: Int): Int {
        var depth = 0
        for (k in open until s.length) {
            if (s[k] == '{') depth++ else if (s[k] == '}' && --depth == 0) return k
        }
        return -1
    }

    private fun choice(s: String): Triple<String, String, List<Pair<String, String>>>? {
        val head = s.split(',', limit = 3)
        if (head.size != 3) return null
        val kind = head[1].trim()
        if (kind !in setOf("plural", "select", "selectordinal")) return null
        var rest = head[2]
        val forms = mutableListOf<Pair<String, String>>()
        while (true) {
            val open = rest.indexOf('{')
            if (open < 0) break
            val end = close(rest, open)
            if (end < 0) return null
            val key = rest.substring(0, open).trim().split(Regex("\\s+")).last() // drops `offset:n`
            forms += key to rest.substring(open + 1, end)
            rest = rest.substring(end + 1)
        }
        return if (forms.isEmpty()) null else Triple(head[0].trim(), kind, forms)
    }
}

/** `String.format` that returns null instead of throwing (a translation with a wrong `%d` can't crash). */
internal fun format(fmt: String, locale: Locale, args: Array<out Any?>): String? =
    if (args.isEmpty() && !fmt.contains('%')) fmt else try {
        String.format(locale, fmt, *args)
    } catch (_: Exception) {
        null
    }

internal fun numberLiteral(arg: Any?): String? = when (arg) {
    is Int, is Long, is Short, is Byte -> arg.toString()
    is Double -> if (arg == Math.floor(arg) && Math.abs(arg) < 1e15) arg.toLong().toString() else arg.toString()
    is Float -> numberLiteral(arg.toDouble())
    else -> null
}

/** Slugs and locale codes go into a URL path and a file name. */
internal fun safe(s: String): Boolean = s.length in 1..64 && s.all { it.isLetterOrDigit() && it.code < 128 || it == '-' || it == '_' }

internal class Response(val code: Int, val etag: String?, val body: ByteArray)

/** The CDN client and disk cache. No user data is sent: two GETs with `If-None-Match`, a generic User-Agent. */
internal class Store(
    val project: String,
    val cdn: String,
    cacheDir: File,
    private val preferred: () -> List<String>,
    private val fetch: (url: String, etag: String?) -> Response? = ::httpGet,
    private val post: (Runnable) -> Unit = { it.run() },
) {
    private val dir = File(File(cacheDir, "glot"), if (safe(project)) project else "_")
    private val listeners = mutableListOf<() -> Unit>()
    private val exec = Executors.newSingleThreadExecutor { r -> Thread(r, "glot-ota").apply { isDaemon = true } }

    @Volatile var forced: String? = null

    @Volatile var locales: List<String> = emptyList()
        private set

    @Volatile var locale: String = "en"
        private set

    @Volatile private var messages: Map<String, String> = emptyMap()

    init {
        // Last downloaded catalog first: the app starts with yesterday's fixes even offline.
        locales = cached("locales.json")?.let(::parseLocales) ?: emptyList()
        locale = pick(forced, locales, preferred())
        messages = cached("$locale.json")?.let(::parseCatalog) ?: emptyMap()
    }

    fun message(key: String): String? = messages[key]

    fun addListener(l: () -> Unit) = synchronized(listeners) { listeners += l }

    fun removeListener(l: () -> Unit) = synchronized(listeners) { listeners -= l }

    fun refreshAsync(done: (Boolean) -> Unit = {}) {
        exec.execute { done(refresh()) }
    }

    /** Re-checks the CDN; true when strings changed. Never throws. */
    @Synchronized
    fun refresh(): Boolean = try {
        locales = get("locales.json")?.first?.let(::parseLocales) ?: locales
        val target = pick(forced, locales, preferred())
        val got = get("$target.json")
        val next = got?.takeIf { it.second || target != locale }?.first?.let(::parseCatalog)
        // null: new language unreachable or unchanged, keep the current strings rather than mix.
        if (next != null && (next != messages || target != locale)) {
            messages = next
            locale = target
            val ls = synchronized(listeners) { listeners.toList() }
            post(Runnable { ls.forEach { it() } })
            true
        } else {
            false
        }
    } catch (_: Exception) {
        false
    }

    /** GET with `If-None-Match`; 304 → cached body. (body, fresh) or null on any failure. */
    private fun get(file: String): Pair<String, Boolean>? {
        if (!safe(project) || !safe(file.removeSuffix(".json"))) return null
        val body = cached(file)
        val tagFile = File(dir, "$file.etag")
        val etag = if (body != null && tagFile.exists()) tagFile.readText() else null
        val res = fetch("${cdn.trimEnd('/')}/cdn/$project/$file", etag) ?: return null
        if (res.code == 304) return body?.let { it to false }
        if (res.code != 200) return null
        val text = res.body.toString(Charsets.UTF_8)
        dir.mkdirs()
        write(File(dir, file), text)
        res.etag?.let { write(tagFile, it) }
        return text to true
    }

    private fun cached(file: String): String? = try {
        File(dir, file).takeIf { it.exists() }?.readText()
    } catch (_: Exception) {
        null
    }

    private fun write(f: File, text: String) {
        val tmp = File(f.parentFile, f.name + ".tmp")
        tmp.writeText(text)
        if (!tmp.renameTo(f)) tmp.delete()
    }

    companion object {
        fun parseCatalog(text: String): Map<String, String>? {
            val root = Json.parse(text) as? Map<*, *> ?: return null
            val out = HashMap<String, String>()
            fun walk(prefix: String, v: Any?) {
                when (v) {
                    is String -> out[prefix] = v
                    is Map<*, *> -> v.forEach { (k, sub) -> walk(if (prefix.isEmpty()) "$k" else "$prefix.$k", sub) }
                }
            }
            walk("", root)
            return out
        }

        fun parseLocales(text: String): List<String>? {
            val list = (Json.parse(text) as? Map<*, *>)?.get("locales") as? List<*> ?: return null
            return list.mapNotNull { e ->
                val m = e as? Map<*, *> ?: return@mapNotNull null
                val code = m["code"] as? String ?: return@mapNotNull null
                code.takeIf { safe(it) && m["ready"] != false }
            }
        }

        /** Forced choice, else the first preferred language glot has ready (exact, then same language). */
        fun pick(forced: String?, available: List<String>, preferred: List<String>): String {
            val norm = { s: String -> s.replace('_', '-').lowercase() }
            for (w in listOfNotNull(forced).ifEmpty { preferred }) {
                available.firstOrNull { norm(it) == norm(w) }?.let { return it }
                available.firstOrNull { norm(it).substringBefore('-') == norm(w).substringBefore('-') }?.let { return it }
            }
            // Not offered (or nothing downloaded yet): the device language; missing keys fall back to strings.xml.
            return forced?.takeIf(::safe) ?: preferred.firstOrNull()?.substringBefore('-')?.takeIf(::safe)
                ?: available.firstOrNull() ?: "en"
        }
    }
}

internal fun httpGet(url: String, etag: String?): Response? = try {
    val c = URL(url).openConnection() as HttpURLConnection
    c.connectTimeout = 15_000
    c.readTimeout = 15_000
    c.useCaches = false
    c.setRequestProperty("User-Agent", "glot-ota") // not Dalvik's, which names the device model
    etag?.let { c.setRequestProperty("If-None-Match", it) }
    try {
        val code = c.responseCode
        Response(code, c.getHeaderField("ETag"), if (code == 200) c.inputStream.use { it.readBytes() } else ByteArray(0))
    } finally {
        c.disconnect()
    }
} catch (_: Exception) {
    null
}

/** Minimal JSON reader (org.json is a stub in JVM unit tests). Null on malformed input. */
internal object Json {
    fun parse(s: String): Any? = try {
        val p = P(s)
        p.value().also { p.ws(); if (p.i != s.length) throw IllegalArgumentException() }
    } catch (_: Exception) {
        null
    }

    private class P(val s: String) {
        var i = 0

        fun ws() {
            while (i < s.length && s[i].isWhitespace()) i++
        }

        fun value(): Any? {
            ws()
            return when (s[i]) {
                '{' -> {
                    i++
                    val m = LinkedHashMap<String, Any?>()
                    ws()
                    if (s[i] == '}') i++ else while (true) {
                        ws()
                        val k = str()
                        ws()
                        expect(':')
                        m[k] = value()
                        ws()
                        if (s[i++] == '}') break else if (s[i - 1] != ',') throw IllegalArgumentException()
                    }
                    m
                }
                '[' -> {
                    i++
                    val l = ArrayList<Any?>()
                    ws()
                    if (s[i] == ']') i++ else while (true) {
                        l += value()
                        ws()
                        if (s[i++] == ']') break else if (s[i - 1] != ',') throw IllegalArgumentException()
                    }
                    l
                }
                '"' -> str()
                't' -> word("true", true)
                'f' -> word("false", false)
                'n' -> word("null", null)
                else -> {
                    val start = i
                    while (i < s.length && s[i] in "+-0123456789.eE") i++
                    s.substring(start, i).toDouble()
                }
            }
        }

        fun expect(c: Char) {
            if (s[i++] != c) throw IllegalArgumentException()
        }

        fun word(w: String, v: Any?): Any? {
            if (!s.startsWith(w, i)) throw IllegalArgumentException()
            i += w.length
            return v
        }

        fun str(): String {
            expect('"')
            val b = StringBuilder()
            while (true) {
                val c = s[i++]
                when (c) {
                    '"' -> return b.toString()
                    '\\' -> when (val e = s[i++]) {
                        'n' -> b.append('\n')
                        't' -> b.append('\t')
                        'r' -> b.append('\r')
                        'b' -> b.append('\b')
                        'f' -> b.append('\u000C')
                        'u' -> b.append(s.substring(i, i + 4).toInt(16).toChar()).also { i += 4 }
                        else -> b.append(e)
                    }
                    else -> b.append(c)
                }
            }
        }
    }
}

/** OTA string → embedded resource (already formatted by Android) → [default] → the key. */
internal fun Store.text(key: String, quantity: Int?, args: Array<out Any?>, default: String?, embedded: () -> String?): String {
    val loc = Locale.forLanguageTag(locale)
    val numbers = quantity?.let { listOf(it.toString()) } ?: args.mapNotNull(::numberLiteral)
    message(key)?.let { m -> format(Icu.expand(m, locale, numbers), loc, args)?.let { return it } }
    try {
        embedded()?.let { return it }
    } catch (_: Exception) {
        // a malformed embedded format: use the default
    }
    val d = default ?: key
    return format(Icu.expand(d, locale, numbers), loc, args) ?: d
}
