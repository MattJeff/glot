# glot on mobile / gettext / Rails catalogs

Each folder is a self-contained project (mock provider, no API key):

| Folder | Format | Files |
|---|---|---|
| `ios` | Apple String Catalog (`xcstrings`) | `App/Localizable.xcstrings` (every locale in one file) |
| `android` | `android` | `res/values-{locale}/strings.xml` (`pt-BR` => `values-pt-rBR`) |
| `flutter` | `arb` | `lib/l10n/app_{locale}.arb` |
| `django` | `po` | `locale/{locale}/LC_MESSAGES/django.po` |
| `rails` | `yaml` | `config/locales/{locale}.yml` |

```sh
cd examples/mobile-formats/android
glot status && glot translate && git diff   # only translated values change
```

Files are patched in place: comments, attributes, order and untouched entries are kept byte for byte.
Plurals are exposed as ICU (`{count, plural, one {%d song} other {%d songs}}`) and written back with the
target locale's CLDR categories; `syntax = "printf"` / `"rails"` validates `%@`, `%1$s`, `%(name)s`, `%{name}`.
xcstrings `comment`, ARB `@key.description`, gettext `#.`/`#:` become translator notes; gettext `fuzzy`
and xcstrings `needs_review` are retranslated as stale. `glot init` detects all of these layouts.
