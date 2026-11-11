package dev.glot.ota

import android.annotation.SuppressLint
import android.content.Context
import android.content.ContextWrapper
import android.content.res.Resources
import android.os.Handler
import android.os.Looper

/**
 * Over-the-air strings from glot's CDN on top of your `strings.xml`.
 *
 * Lookup: the downloaded catalog for [locale], then your resources (`res/values-xx/strings.xml`), then the
 * default. The language follows the app's configuration (so `AppCompatDelegate.setApplicationLocales`)
 * and can be any language glot has ready, embedded or not. Nothing about the user is sent; offline or a
 * CDN outage only means the last downloaded catalog (or your resources) keeps being used.
 */
object Glot {
    @Volatile private var store: Store? = null
    @Volatile private var app: Context? = null

    /** [project]: the project slug (its CDN must be public). [cdn]: your glot server, e.g. `https://api.example.com`. */
    @JvmStatic
    @JvmOverloads
    fun configure(context: Context, project: String, cdn: String, locale: String? = null) {
        val ctx = context.applicationContext ?: context
        app = ctx
        val main = Handler(Looper.getMainLooper())
        store = Store(project, cdn, ctx.cacheDir, preferred = { languages(ctx) }, post = { main.post(it) })
            .also { it.forced = locale; it.refreshAsync() }
    }

    /** `getString` with OTA first. [args] follow `String.format` (`%1$s`, `%d`). */
    @JvmStatic
    fun string(key: String, default: String? = null, vararg args: Any?): String =
        resolve(key, null, args, default) {
            embedded(key, "string") { id, r -> if (args.isEmpty()) r.getString(id) else r.getString(id, *args) }
        }

    /** `getQuantityString` with OTA first: the plural form for [quantity] (CLDR rules), then [args]. */
    @JvmStatic
    fun plural(key: String, quantity: Int, default: String? = null, vararg args: Any?): String =
        resolve(key, quantity, args, default) {
            embedded(key, "plurals") { id, r -> r.getQuantityString(id, quantity, *args) }
        }

    internal fun has(key: String) = store?.message(key) != null

    internal fun resolve(key: String, quantity: Int?, args: Array<out Any?>, default: String?, embedded: () -> String?): String =
        store?.text(key, quantity, args, default, embedded)
            ?: (try { embedded() } catch (_: Exception) { null }) ?: default ?: key

    /** Re-checks the CDN in the background (e.g. from `onResume`); [done] gets true when strings changed. */
    @JvmStatic
    @JvmOverloads
    fun refresh(done: (Boolean) -> Unit = {}) {
        store?.refreshAsync(done)
    }

    /** Force a language (null: follow the app configuration again), downloading it if needed. */
    @JvmStatic
    @JvmOverloads
    fun setLocale(code: String?, done: (Boolean) -> Unit = {}) {
        store?.forced = code
        refresh(done)
    }

    /** Called on the main thread after new strings arrived: re-render (recreate(), or bump Compose state). */
    @JvmStatic fun addListener(l: () -> Unit) { store?.addListener(l) }

    @JvmStatic fun removeListener(l: () -> Unit) { store?.removeListener(l) }

    /** The language strings are served in. */
    @JvmStatic val locale: String get() = store?.locale ?: "en"

    /** Languages glot has ready, source first: offer them in a picker, embedded in the APK or not. */
    @JvmStatic val locales: List<String> get() = store?.locales ?: emptyList()

    /**
     * `Context.getString(R.string.x)` with OTA strings: in your Activity,
     * `override fun attachBaseContext(base: Context) = super.attachBaseContext(Glot.wrap(base))`.
     * With AppCompat also `override fun getResources() = Glot.wrap(super.getResources())`.
     * Layout XML (`android:text="@string/x"`) is inflated through TypedArray and is not intercepted: set those
     * texts from code.
     */
    @JvmStatic
    fun wrap(base: Context): Context = object : ContextWrapper(base) {
        private val res by lazy { wrap(base.resources) }
        override fun getResources(): Resources = res
    }

    @JvmStatic
    fun wrap(base: Resources): Resources = if (base is GlotResources) base else GlotResources(base)

    @SuppressLint("DiscouragedApi") // by-name lookup is the point: keys are resource names
    private fun embedded(key: String, type: String, get: (Int, Resources) -> String): String? {
        val ctx = app ?: return null
        val id = ctx.resources.getIdentifier(key, type, ctx.packageName)
        return if (id == 0) null else get(id, ctx.resources)
    }

    private fun languages(ctx: Context): List<String> {
        val l = ctx.resources.configuration.locales
        return (0 until l.size()).map { l[it].toLanguageTag() }
    }
}

@Suppress("DEPRECATION") // the only way to hand a Resources to framework code
internal class GlotResources(private val base: Resources) :
    Resources(base.assets, base.displayMetrics, base.configuration) {
    private fun name(id: Int): String? = try {
        if (getResourcePackageName(id) == "android") null else getResourceEntryName(id)
    } catch (_: NotFoundException) {
        null
    }

    /** The OTA value when glot has one; null lets the base Resources answer (keeping styled text). */
    private fun ota(id: Int, quantity: Int?, args: Array<out Any?>): String? {
        val key = name(id)?.takeIf { Glot.has(it) } ?: return null
        return Glot.resolve(key, quantity, args, null) {
            when {
                quantity != null -> base.getQuantityString(id, quantity, *args)
                args.isEmpty() -> base.getString(id)
                else -> base.getString(id, *args)
            }
        }
    }

    override fun getText(id: Int): CharSequence = ota(id, null, emptyArray()) ?: base.getText(id)
    override fun getString(id: Int): String = ota(id, null, emptyArray()) ?: base.getString(id)
    override fun getString(id: Int, vararg formatArgs: Any?): String = ota(id, null, formatArgs) ?: base.getString(id, *formatArgs)
    override fun getQuantityText(id: Int, quantity: Int): CharSequence = ota(id, quantity, emptyArray()) ?: base.getQuantityText(id, quantity)
    override fun getQuantityString(id: Int, quantity: Int): String = ota(id, quantity, emptyArray()) ?: base.getQuantityString(id, quantity)
    override fun getQuantityString(id: Int, quantity: Int, vararg formatArgs: Any?): String =
        ota(id, quantity, formatArgs) ?: base.getQuantityString(id, quantity, *formatArgs)
}
