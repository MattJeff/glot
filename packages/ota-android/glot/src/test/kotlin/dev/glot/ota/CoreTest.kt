package dev.glot.ota

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.util.Locale

class CoreTest {
    private val locales = """{"source":"en","locales":[{"code":"en","ready":true},{"code":"fr","ready":true},{"code":"pl","ready":true},{"code":"de","ready":false}]}"""
    private val fr = """{"home":{"title":"Bonjour"},"items":"{count, plural, one {%d élément} other {%d éléments}}","bad":"%d x","greet":"Salut %1${'$'}s"}"""
    private val pl = """{"items":"{count, plural, one {%d element} few {%d elementy} many {%d elementów} other {%d elementu}}"}"""

    /** Fake CDN with ETags / 304s; records (url, If-None-Match). */
    private class Cdn(var files: Map<String, String>) {
        val calls = mutableListOf<Pair<String, String?>>()
        var down = false

        fun fetch(url: String, etag: String?): Response? {
            calls += url to etag
            if (down) return null
            val body = files[url.substringAfterLast('/')] ?: return Response(404, null, ByteArray(0))
            val tag = "\"${body.hashCode()}\""
            return if (etag == tag) Response(304, tag, ByteArray(0)) else Response(200, tag, body.toByteArray())
        }
    }

    private fun tmp(): File = Files.createTempDirectory("glot").toFile()

    private fun store(dir: File, cdn: Cdn, preferred: List<String> = listOf("fr-FR")) =
        Store("demo", "https://glot.test/", dir, { preferred }, cdn::fetch)

    @Test
    fun pluralRulesMatchGlotValidator() {
        @Suppress("UNCHECKED_CAST")
        val fixture = Json.parse(File("../../ota-fixtures/plural-cases.json").readText()) as Map<String, Any>
        val numbers = fixture["numbers"] as List<String>
        val cases = fixture["categories"] as Map<String, List<String>>
        val wrong = cases.flatMap { (loc, want) ->
            numbers.zip(want).filter { (n, w) -> Plural.category(loc, n) != w }.map { (n, w) -> "$loc $n: ${Plural.category(loc, n)} != $w" }
        }
        assertTrue(cases.size > 150)
        assertEquals(emptyList<String>(), wrong.take(20))
    }

    @Test
    fun icuPluralsAndSelects() {
        val ru = "{count, plural, one {%d файл} few {%d файла} many {%d файлов} other {%d файла}}"
        assertEquals("%d файл", Icu.expand(ru, "ru", listOf("21")))
        assertEquals("%d файлов", Icu.expand(ru, "ru", listOf("11")))
        assertEquals("Empty", Icu.expand("{count, plural, =0 {Empty} other {# items}}", "en", listOf("0")))
        assertEquals("7 items", Icu.expand("{count, plural, =0 {Empty} other {# items}}", "en", listOf("7")))
        assertEquals("B", Icu.expand("{g, select, a {A} b {B} other {O}}", "en", emptyList(), mapOf("g" to "b")))
        assertEquals("Literal {name} {", Icu.expand("Literal {name} {", "en", emptyList()))
    }

    @Test
    fun formatNeverThrows() {
        assertEquals("Ana a 3", format("%1\$s a %2\$d", Locale.FRENCH, arrayOf("Ana", 3)))
        assertNull(format("%d", Locale.FRENCH, arrayOf("x")))
        assertNull(format("%d %d", Locale.FRENCH, arrayOf(1)))
        assertEquals("100%", format("100%%", Locale.FRENCH, emptyArray()))
    }

    @Test
    fun downloadsCachesRevalidatesAndSurvivesOffline() {
        val dir = tmp()
        val cdn = Cdn(mapOf("locales.json" to locales, "fr.json" to fr))
        val s = store(dir, cdn)
        var notified = 0
        s.addListener { notified++ }
        assertEquals("Hello", s.text("home.title", null, emptyArray(), "Hello") { null })
        assertTrue(s.refresh())
        assertEquals(1, notified)
        assertEquals("fr", s.locale)
        assertEquals(listOf("en", "fr", "pl"), s.locales) // de not ready
        assertEquals("Bonjour", s.text("home.title", null, emptyArray(), "Hello") { null })
        assertEquals("1 élément", s.text("items", 1, arrayOf(1), null) { null })
        assertEquals("5 éléments", s.text("items", 5, arrayOf(5), null) { null })
        assertEquals("Salut Ana", s.text("greet", null, arrayOf("Ana"), null) { null })

        cdn.calls.clear()
        assertFalse(s.refresh()) // 304s
        assertEquals(listOf("https://glot.test/cdn/demo/locales.json", "https://glot.test/cdn/demo/fr.json"), cdn.calls.map { it.first })
        assertTrue(cdn.calls.all { it.second != null })
        assertEquals(1, notified)

        // Restart offline: the cached catalog is served right away.
        cdn.down = true
        val again = store(dir, cdn)
        assertEquals("Bonjour", again.text("home.title", null, emptyArray(), "Hello") { null })
        assertFalse(again.refresh())
        assertEquals("Bonjour", again.text("home.title", null, emptyArray(), "Hello") { null })
    }

    @Test
    fun newLanguageWithoutAnUpdate() {
        val s = store(tmp(), Cdn(mapOf("locales.json" to locales, "fr.json" to fr, "pl.json" to pl)))
        s.refresh()
        s.forced = "pl-PL"
        assertTrue(s.refresh())
        assertEquals("pl", s.locale)
        assertEquals("3 elementy", s.text("items", 3, arrayOf(3), null) { null })
        assertEquals("5 elementów", s.text("items", 5, arrayOf(5), null) { null })
        assertEquals("fr", Store.pick(null, listOf("en", "fr"), listOf("fr-CA")))
        assertEquals("de", Store.pick(null, listOf("en", "fr"), listOf("de-DE"))) // device language, strings.xml fills in
    }

    @Test
    fun fallsBackToResourcesThenDefault() {
        for (cdn in listOf(Cdn(emptyMap()), Cdn(mapOf("locales.json" to "{", "fr.json" to "nope")), Cdn(emptyMap()).apply { down = true })) {
            val s = store(tmp(), cdn)
            assertFalse(s.refresh())
            assertEquals("From strings.xml", s.text("shipped", null, emptyArray(), "x") { "From strings.xml" })
            assertEquals("Default 2", s.text("missing", null, arrayOf(2), "Default %d") { null })
            assertEquals("missing", s.text("missing", null, emptyArray(), null) { null })
            assertEquals("x", s.text("boom", null, emptyArray(), "x") { throw IllegalStateException() })
        }
    }

    @Test
    fun badTranslationPlaceholderFallsBack() {
        val s = store(tmp(), Cdn(mapOf("locales.json" to locales, "fr.json" to fr)))
        s.refresh()
        // "bad" is "%d x" on the CDN; fed a String it would throw: the embedded string is used.
        assertEquals("Ana y", s.text("bad", null, arrayOf("Ana"), null) { "Ana y" })
    }

    @Test
    fun jsonAndSafety() {
        assertEquals(mapOf("a.b" to "x", "c" to "é\n"), Store.parseCatalog("""{"a":{"b":"x"},"c":"é\n","n":1}"""))
        assertNull(Store.parseCatalog("[1"))
        assertEquals(listOf("en"), Store.parseLocales("""{"locales":[{"code":"en"},{"code":"../etc","ready":true}]}"""))
        assertFalse(safe("../x"))
    }
}
