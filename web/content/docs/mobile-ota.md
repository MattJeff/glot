# Mobile apps (OTA)

A wording fix or a new language normally means a new build, store review and waiting for users to update. The glot OTA SDKs download your strings from glot's CDN at runtime and lay them over the ones compiled into the app. Fixes and new languages reach users on the next launch, without a store release.

- **Embedded fallback.** Your String Catalog, `strings.xml`, ARB files or bundled JSON stay the source of truth. A key glot doesn't have, a network failure, a CDN outage or a malformed response all fall back to them. Nothing throws.
- **Cached on disk.** The last downloaded catalog is reused at startup, offline too. Updates use `ETag` / `If-None-Match`, so an unchanged catalog costs a `304` and no body.
- **Native formats.** Placeholders and plurals stay in each platform's own syntax (`%@`/`%lld`, `%1$s`/`%d`, `{name}`, `{{count}}`). Plural forms follow the CLDR rules glot's validator uses. The Swift and Kotlin SDKs are tested against the validator's output for 154 languages.
- **No user data.** The SDKs only make two GET requests: `/cdn/{project}/locales.json` and `/cdn/{project}/{locale}.json`. They send no cookies and no identifiers, with a generic `User-Agent` on iOS, Android and Flutter.

Requirements: the project's CDN must be public (**Settings → Public CDN**), and the app uses the same keys as the catalog glot syncs (`glot push` / the GitHub app do that).

## How a string is resolved

1. The downloaded catalog for the current language.
2. The string embedded in the app (Bundle, Resources, your gen-l10n getter, bundled JSON).
3. The default you pass, then the key itself.

A downloaded translation is only used if it formats with the arguments you pass. On iOS a `%@` given an `Int` would crash `String(format:)`, and on Android a bad `%d` throws. In both cases the SDK falls back to step 2.

**Language choice:** the first of the user's preferred languages that glot has *ready* (see `locales.json`), exact match first, then same language (`fr-CA` → `fr`). Otherwise the device language, with any missing keys filled in from the app. `Glot.locales` lists the ready languages, so a language picker can offer languages the binary doesn't embed.

## iOS / macOS (Swift Package)

Add `packages/ota-ios` as a local package (Xcode: *File → Add Package Dependencies → Add Local…*). SwiftPM can't load a package from a subfolder of a Git repository, so copy or vendor the folder.

```swift
import Glot

// App start
Glot.configure(project: "my-app", cdn: URL(string: "https://api.example.com")!)

// Anywhere
label.text = Glot.string("home.title", default: "Welcome")
Text(Glot.string("%lld items", default: "%lld items", count))
```

- Fallback: `Bundle.main.localizedString(forKey:)`. Your `.xcstrings` (compiled to `.strings`/`.stringsdict` by Xcode) answers any key glot doesn't have.
- Plural and device variations are served as ICU by glot and resolved on the device. A plural block takes the numeric arguments in order, so `%#@files@ in %#@folders@` reads argument 1, then argument 2. Device variations pick `iphone`, `ipad`, `mac`, `appletv`, `applewatch` or `applevision`.
- Updates: `configure` checks the CDN in the background. Call `await Glot.refresh()` when the app becomes active. `.glotDidUpdate` is posted on the main thread when strings changed:

```swift
.onReceive(NotificationCenter.default.publisher(for: .glotDidUpdate)) { _ in refreshID = UUID() }
```

- `await Glot.setLocale("pl")` switches language (and downloads it). `nil` goes back to the system preference.

## Android (Kotlin)

Include `packages/ota-android/glot` as a module (`include(":glot")` with `project(":glot").projectDir = file("…/ota-android/glot")`), then add `implementation(project(":glot"))`. It needs minSdk 24 and has no dependencies.

```kotlin
class App : Application() {
    override fun onCreate() {
        super.onCreate()
        Glot.configure(this, project = "my-app", cdn = "https://api.example.com")
    }
}

Glot.string("welcome", default = "Welcome")
Glot.plural("items", quantity = n, default = "%d items", n)
```

- Fallback: `strings.xml` / `<plurals>` by resource name (`getString` / `getQuantityString`).
- To serve `context.getString(R.string.x)` from glot, wrap the context:

```kotlin
override fun attachBaseContext(base: Context) = super.attachBaseContext(Glot.wrap(base))
// AppCompatActivity, also:
override fun getResources(): Resources = Glot.wrap(super.getResources())
```

  Text inflated from layout XML (`android:text="@string/x"`) goes through `TypedArray` and isn't intercepted. Set those texts in code.
- The language follows the app configuration, including `AppCompatDelegate.setApplicationLocales`, so per-app language works as is. It can also be any language glot has ready. `Glot.setLocale("pl")` forces one.
- Updates: `configure` checks in the background. Call `Glot.refresh()` from `onResume`. `Glot.addListener { recreate() }` runs on the main thread when strings changed.

## Flutter (gen-l10n / ARB)

```yaml
dependencies:
  glot_ota:
    git: { url: https://github.com/MattJeff/glot, path: packages/ota-flutter }
```

```dart
await GlotOta.configure(project: 'my-app', cdn: Uri.parse('https://api.example.com'));

ListenableBuilder(
  listenable: GlotOta.instance,
  builder: (context, _) => MaterialApp(
    localizationsDelegates: [GlotOta.delegate, ...AppLocalizations.localizationsDelegates],
    supportedLocales: GlotOta.supportedLocales(AppLocalizations.supportedLocales),
  ),
);

// In a widget
final g = Glot.of(context);
Text(g.t('itemCount', args: {'count': n}, orElse: () => AppLocalizations.of(context)?.itemCount(n)));
```

- `GlotOta.delegate` is a `LocalizationsDelegate`. It loads the cached catalog for the locale Flutter picked, then checks the CDN. When strings change, the `ListenableBuilder` rebuilds and the new delegate reloads.
- ARB messages are formatted on the device: `{name}`, `plural` (through `Intl.plural`'s logic, as generated code does, with `=0`/`=1`/`=2`) and `select`.
- `orElse` is your generated getter: gen-l10n stays the fallback. For a language your build doesn't embed, `AppLocalizations.of(context)` is `null`, hence the `?.`. Flutter's Material and Cupertino localizations must support that language (about 115 do).

## React Native / Expo (i18next)

```sh
npm i ./packages/ota-react-native @react-native-async-storage/async-storage
```

```ts
import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { GlotOtaBackend } from "@glot-i18n/ota-react-native";
import en from "./locales/en/translation.json";
import fr from "./locales/fr/translation.json";

i18next
  .use(GlotOtaBackend)
  .use(initReactI18next)
  .init({
    lng: "fr",
    fallbackLng: "en",
    react: { bindI18nStore: "added" }, // re-render when newer strings arrive
    backend: {
      project: "my-app",
      cdn: "https://api.example.com",
      storage: AsyncStorage,
      bundled: { en: { translation: en }, fr: { translation: fr } },
    },
  });
```

- The backend answers right away with the bundled JSON merged with the cached catalog. Then it checks the CDN and adds newer strings with `addResourceBundle`. Only a language the app doesn't bundle, never downloaded before, waits for the network.
- Plurals (`key_one`, `key_few`…) and interpolation (`{{name}}`) are i18next's own, as glot stores them for i18next projects.
- `backend.locales()` (`i18next.services.backendConnector.backend.locales()`) lists ready languages for a picker.

## Limits

- `selectordinal` takes the `other` form (exact `=N` cases still match). Cardinal plurals are complete.
- iOS: plural blocks take numeric arguments in order. A String Catalog substitution whose `argNum` isn't in that order picks the wrong number. The formatting check still prevents a crash.
- Android: layout XML `@string/` references aren't intercepted. Neither is Compose's `stringResource()`: use `Glot.string(...)` there.
- Flutter: strings go through `Glot.of(context).t(...)`, not through the generated getters directly.
- A private project's CDN needs a token, which a shipped app can't hold safely. Use a public CDN.
- Not published to CocoaPods, Maven, pub.dev or npm yet. Use the source folders above.
