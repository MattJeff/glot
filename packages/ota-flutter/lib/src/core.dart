// Pure Dart part (no Flutter): CDN + disk cache, locale choice, ARB/ICU formatting.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:intl/intl.dart';

class GlotResponse {
  GlotResponse(this.status, this.etag, this.body);
  final int status;
  final String? etag;
  final String body;
}

/// GET [url] with `If-None-Match: etag`; null on network failure.
typedef GlotFetch = Future<GlotResponse?> Function(Uri url, String? etag);

/// No user data is sent: two GETs with `If-None-Match` and a generic User-Agent, no cookies.
Future<GlotResponse?> httpFetch(Uri url, String? etag) async {
  final client = HttpClient()
    ..connectionTimeout = const Duration(seconds: 15)
    ..userAgent = 'glot-ota';
  try {
    final req = await client.getUrl(url);
    if (etag != null) req.headers.set(HttpHeaders.ifNoneMatchHeader, etag);
    final res = await req.close().timeout(const Duration(seconds: 15));
    final body = res.statusCode == 200 ? await res.transform(utf8.decoder).join() : '';
    return GlotResponse(res.statusCode, res.headers.value(HttpHeaders.etagHeader), body);
  } catch (_) {
    return null;
  } finally {
    client.close(force: true);
  }
}

bool _safe(String s) => RegExp(r'^[A-Za-z0-9_-]{1,64}$').hasMatch(s);

class GlotCore {
  GlotCore({required this.project, required this.cdn, required this.dir, GlotFetch? fetch})
      : _fetch = fetch ?? httpFetch;

  final String project;
  final Uri cdn;
  final Directory dir;
  final GlotFetch _fetch;

  /// Ready languages from `locales.json` (source first).
  List<String> locales = const [];
  String? locale;
  Map<String, String> messages = const {};

  Directory get _root => Directory('${dir.path}/glot/${_safe(project) ? project : '_'}');

  /// Cached `locales.json` and the catalog for [wanted]: the app starts with the last fixes, offline too.
  void loadCached(String wanted) {
    locales = _parseLocales(_read('locales.json')) ?? locales;
    final code = pick(wanted, locales);
    final cached = _parseCatalog(_read('$code.json'));
    if (cached != null || code != locale) {
      locale = code;
      messages = cached ?? const {};
    }
  }

  /// Re-checks the CDN for [wanted]; true when strings changed. Never throws.
  Future<bool> refresh(String wanted) async {
    try {
      final list = await _get('locales.json');
      locales = _parseLocales(list?.$1) ?? locales;
      final code = pick(wanted, locales);
      final got = await _get('$code.json');
      if (got == null || (!got.$2 && code == locale)) return false;
      final next = _parseCatalog(got.$1);
      if (next == null) return false;
      final changed = code != locale || !_same(next, messages);
      locale = code;
      messages = next;
      return changed;
    } catch (_) {
      return false;
    }
  }

  /// (body, fresh) — 304 gives the cached body; null on any failure.
  Future<(String, bool)?> _get(String file) async {
    if (!_safe(project) || !_safe(file.replaceAll('.json', ''))) return null;
    final cached = _read(file);
    final tag = File('${_root.path}/$file.etag');
    final etag = cached != null && tag.existsSync() ? tag.readAsStringSync() : null;
    final base = cdn.toString().replaceAll(RegExp(r'/+$'), '');
    final res = await _fetch(Uri.parse('$base/cdn/$project/$file'), etag);
    if (res == null) return null;
    if (res.status == 304) return cached == null ? null : (cached, false);
    if (res.status != 200) return null;
    try {
      _root.createSync(recursive: true);
      _write(File('${_root.path}/$file'), res.body);
      if (res.etag != null) _write(tag, res.etag!);
    } catch (_) {}
    return (res.body, true);
  }

  String? _read(String file) {
    try {
      final f = File('${_root.path}/$file');
      return f.existsSync() ? f.readAsStringSync() : null;
    } catch (_) {
      return null;
    }
  }

  void _write(File f, String text) {
    final tmp = File('${f.path}.tmp')..writeAsStringSync(text);
    tmp.renameSync(f.path);
  }

  /// First match of [wanted] among ready languages (exact, then same language), else [wanted] itself:
  /// keys glot lacks fall back to the app's ARB.
  static String pick(String wanted, List<String> available) {
    String norm(String s) => s.replaceAll('_', '-').toLowerCase();
    String base(String s) => norm(s).split('-').first;
    for (final a in available) {
      if (norm(a) == norm(wanted)) return a;
    }
    for (final a in available) {
      if (base(a) == base(wanted)) return a;
    }
    return _safe(wanted) ? wanted.replaceAll('_', '-') : 'en';
  }

  static Map<String, String>? _parseCatalog(String? text) {
    if (text == null) return null;
    try {
      final root = jsonDecode(text);
      if (root is! Map) return null;
      final out = <String, String>{};
      void walk(String prefix, Object? v) {
        if (v is String) out[prefix] = v;
        if (v is Map) v.forEach((k, sub) => walk(prefix.isEmpty ? '$k' : '$prefix.$k', sub));
      }

      walk('', root);
      return out;
    } catch (_) {
      return null;
    }
  }

  static List<String>? _parseLocales(String? text) {
    if (text == null) return null;
    try {
      final list = (jsonDecode(text) as Map)['locales'] as List;
      return [
        for (final e in list)
          if (e is Map && e['code'] is String && _safe(e['code'] as String) && e['ready'] != false)
            e['code'] as String,
      ];
    } catch (_) {
      return null;
    }
  }

  static bool _same(Map<String, String> a, Map<String, String> b) =>
      a.length == b.length && a.entries.every((e) => b[e.key] == e.value);
}

/// ARB message (ICU: `{name}`, `{n, plural, =0{…} one{…} other{…}}`, `{g, select, …}`) with [args].
/// Plurals go through `Intl.plural`'s logic exactly as gen-l10n's generated code does (`=0`/`=1`/`=2`
/// count as zero/one/two). Unknown arguments are left as written; never throws.
String formatMessage(String msg, Map<String, Object?> args, String locale) {
  try {
    return _expand(msg, args, locale, null);
  } catch (_) {
    return msg;
  }
}

String _expand(String s, Map<String, Object?> args, String locale, num? hash) {
  final out = StringBuffer();
  var i = 0;
  while (i < s.length) {
    final ch = s[i];
    if (ch == '#' && hash != null) {
      out.write(_num(hash));
      i++;
      continue;
    }
    final end = ch == '{' ? _close(s, i) : -1;
    if (end < 0) {
      out.write(ch);
      i++;
      continue;
    }
    final inner = s.substring(i + 1, end);
    final head = inner.split(',');
    final name = head.first.trim();
    if (head.length == 1) {
      out.write(args.containsKey(name) ? '${args[name] is num ? _num(args[name] as num) : args[name]}' : '{$inner}');
    } else {
      final kind = head[1].trim();
      final forms = _forms(inner.substring(head[0].length + head[1].length + 2));
      final value = args[name];
      if (kind == 'select') {
        final body = forms['$value'] ?? forms['other'];
        out.write(body == null ? '' : _expand(body, args, locale, hash));
      } else if (kind == 'plural' && value is num) {
        String? f(String k) => forms[k];
        final body = Intl.pluralLogic<String?>(value,
            locale: locale,
            zero: f('=0') ?? f('zero'),
            one: f('=1') ?? f('one'),
            two: f('=2') ?? f('two'),
            few: f('few'),
            many: f('many'),
            other: f('other') ?? forms.values.first);
        out.write(body == null ? '' : _expand(body, args, locale, value));
      } else {
        out.write('{$inner}');
      }
    }
    i = end + 1;
  }
  return out.toString();
}

String _num(num n) => n is double && n == n.truncateToDouble() && n.abs() < 1e15 ? '${n.toInt()}' : '$n';

int _close(String s, int open) {
  var depth = 0;
  for (var k = open; k < s.length; k++) {
    if (s[k] == '{') depth++;
    if (s[k] == '}' && --depth == 0) return k;
  }
  return -1;
}

Map<String, String> _forms(String rest) {
  final forms = <String, String>{};
  while (true) {
    final open = rest.indexOf('{');
    if (open < 0) break;
    final end = _close(rest, open);
    if (end < 0) break;
    final key = rest.substring(0, open).trim().split(RegExp(r'\s+')).last;
    forms[key] = rest.substring(open + 1, end);
    rest = rest.substring(end + 1);
  }
  return forms;
}
