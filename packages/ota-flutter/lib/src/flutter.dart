import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:path_provider/path_provider.dart';

import 'core.dart';

/// Over-the-air strings from glot's CDN on top of your ARB files (gen-l10n).
///
/// ```dart
/// await GlotOta.configure(project: 'my-app', cdn: Uri.parse('https://api.example.com'));
/// ListenableBuilder(
///   listenable: GlotOta.instance,
///   builder: (context, _) => MaterialApp(
///     localizationsDelegates: [GlotOta.delegate, ...AppLocalizations.localizationsDelegates],
///     supportedLocales: GlotOta.supportedLocales(AppLocalizations.supportedLocales),
///   ),
/// );
/// // in a widget:
/// Text(Glot.of(context).t('itemCount', args: {'count': n}, orElse: () => AppLocalizations.of(context)?.itemCount(n)))
/// ```
/// Lookup: the downloaded catalog, then `orElse` (your generated getter), then the key. A network or CDN
/// failure only means the last downloaded catalog (or your ARB) keeps being used.
class GlotOta extends ChangeNotifier {
  GlotOta._();

  static final GlotOta instance = GlotOta._();

  GlotCore? _core;
  String? _wanted;
  int _version = 0;

  /// [project]: the project slug (its CDN must be public). [cdn]: your glot server.
  static Future<void> configure({
    required String project,
    required Uri cdn,
    Directory? cacheDirectory,
    GlotFetch? fetch,
  }) async {
    Directory dir;
    try {
      dir = cacheDirectory ?? await getApplicationSupportDirectory();
    } catch (_) {
      dir = Directory.systemTemp;
    }
    instance._core = GlotCore(project: project, cdn: cdn, dir: dir, fetch: fetch)
      ..loadCached(Platform.localeName); // cached `locales.json` for supportedLocales at startup
  }

  /// The delegate to list first in `localizationsDelegates`; a new instance after each update, so that
  /// `MaterialApp` rebuilt by a `ListenableBuilder` reloads the strings.
  static LocalizationsDelegate<Glot> get delegate => _GlotDelegate(instance._version);

  /// [embedded] plus every language glot has ready: new languages without a store release.
  static List<Locale> supportedLocales(Iterable<Locale> embedded) {
    final out = [...embedded];
    for (final code in instance._core?.locales ?? const <String>[]) {
      final parts = code.split(RegExp('[-_]'));
      final script = parts.length > 1 && parts[1].length == 4 ? parts[1] : null;
      final region = parts.length > 1 && parts.last.length != 4 ? parts.last : null;
      final l = Locale.fromSubtags(languageCode: parts.first, scriptCode: script, countryCode: region);
      if (!out.any((e) => e.toLanguageTag() == l.toLanguageTag())) out.add(l);
    }
    return out;
  }

  /// Re-checks the CDN (e.g. on `AppLifecycleState.resumed`); true when strings changed.
  Future<bool> refresh() async {
    final core = _core, wanted = _wanted;
    if (core == null || wanted == null) return false;
    final changed = await core.refresh(wanted);
    if (changed) {
      _version++;
      notifyListeners();
    }
    return changed;
  }

  Future<Glot> _load(Locale locale) async {
    final core = _core;
    _wanted = locale.toLanguageTag();
    if (core == null) return Glot._({}, _wanted!);
    core.loadCached(_wanted!);
    // ponytail: first launch of a language shows the ARB until the download lands, then rebuilds once.
    refresh();
    return Glot._(core.messages, core.locale ?? _wanted!);
  }
}

/// The strings for the current locale: `Glot.of(context).t('key')`.
class Glot {
  Glot._(this._messages, this.locale);

  final Map<String, String> _messages;

  /// The language glot serves (may differ from the app locale's region: `fr-CA` → `fr`).
  final String locale;

  static Glot of(BuildContext context) => Localizations.of<Glot>(context, Glot) ?? Glot._({}, 'en');

  /// The glot translation of [key] formatted with [args] (ARB placeholders and plurals), else [orElse]
  /// (pass your gen-l10n getter), else the key.
  String t(String key, {Map<String, Object?> args = const {}, String? Function()? orElse}) {
    final m = _messages[key];
    if (m != null) return formatMessage(m, args, locale);
    try {
      return orElse?.call() ?? key;
    } catch (_) {
      return key;
    }
  }
}

class _GlotDelegate extends LocalizationsDelegate<Glot> {
  const _GlotDelegate(this.version);

  final int version;

  @override
  bool isSupported(Locale locale) => true;

  @override
  Future<Glot> load(Locale locale) => GlotOta.instance._load(locale);

  @override
  bool shouldReload(_GlotDelegate old) => old.version != version;
}
