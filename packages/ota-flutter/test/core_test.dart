import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:glot_ota/src/core.dart';

const locales =
    '{"source":"en","locales":[{"code":"en","ready":true},{"code":"fr","ready":true},{"code":"pl","ready":true},{"code":"de","ready":false}]}';
const fr = '{"home":{"title":"Bonjour"},"itemCount":"{count, plural, =0{Aucun élément} one{{count} élément} other{{count} éléments}}","hello":"Salut {name}"}';
const pl = '{"itemCount":"{count, plural, one{{count} element} few{{count} elementy} many{{count} elementów} other{{count} elementu}}"}';

/// Fake CDN with ETags / 304s; records (url, If-None-Match).
class Cdn {
  Cdn(this.files);
  Map<String, String> files;
  bool down = false;
  final calls = <(String, String?)>[];

  Future<GlotResponse?> fetch(Uri url, String? etag) async {
    calls.add((url.toString(), etag));
    if (down) return null;
    final body = files[url.pathSegments.last];
    if (body == null) return GlotResponse(404, null, '');
    final tag = '"${body.hashCode}"';
    return etag == tag ? GlotResponse(304, tag, '') : GlotResponse(200, tag, body);
  }
}

GlotCore core(Directory dir, Cdn cdn) =>
    GlotCore(project: 'demo', cdn: Uri.parse('https://glot.test/'), dir: dir, fetch: cdn.fetch);

Directory tmp() => Directory.systemTemp.createTempSync('glot');

void main() {
  test('ARB plurals, selects and placeholders as gen-l10n does', () {
    const m = '{count, plural, =0{none} one{{count} file} other{{count} files}}';
    expect(formatMessage(m, {'count': 0}, 'en'), 'none');
    expect(formatMessage(m, {'count': 1}, 'en'), '1 file');
    expect(formatMessage(m, {'count': 5}, 'en'), '5 files');
    const ru = '{n, plural, one{# файл} few{# файла} many{# файлов} other{# файла}}';
    expect(formatMessage(ru, {'n': 21}, 'ru'), '21 файл');
    expect(formatMessage(ru, {'n': 3}, 'ru'), '3 файла');
    expect(formatMessage(ru, {'n': 11}, 'ru'), '11 файлов');
    expect(formatMessage('{g, select, female{Elle} other{Il}} {name}', {'g': 'female', 'name': 'Ana'}, 'fr'), 'Elle Ana');
    expect(formatMessage('Hi {missing} {', {}, 'en'), 'Hi {missing} {');
  });

  test('plural categories agree with glot\'s validator on integers', () {
    final fixture = jsonDecode(File('../ota-fixtures/plural-cases.json').readAsStringSync()) as Map;
    final numbers = (fixture['numbers'] as List).cast<String>();
    final cats = fixture['categories'] as Map;
    for (final loc in ['en', 'fr', 'de', 'ru', 'pl', 'cs', 'ar', 'ja', 'lt', 'cy']) {
      final want = (cats[loc] as List).cast<String>();
      for (var k = 0; k < numbers.length; k++) {
        final n = int.tryParse(numbers[k]);
        if (n == null || n <= 2) continue; // 0/1/2 are explicit cases in gen-l10n semantics
        const forms = {'zero': 'zero', 'one': 'one', 'two': 'two', 'few': 'few', 'many': 'many', 'other': 'other'};
        final msg = '{n, plural, ${forms.entries.map((e) => '${e.key}{${e.value}}').join(' ')}}';
        expect(formatMessage(msg, {'n': n}, loc), want[k], reason: '$loc $n');
      }
    }
  });

  test('downloads, caches, revalidates and survives offline', () async {
    final dir = tmp();
    final cdn = Cdn({'locales.json': locales, 'fr.json': fr});
    final c = core(dir, cdn);
    expect(await c.refresh('fr-FR'), isTrue);
    expect(c.locale, 'fr');
    expect(c.locales, ['en', 'fr', 'pl']);
    expect(c.messages['home.title'], 'Bonjour');
    expect(formatMessage(c.messages['itemCount']!, {'count': 0}, 'fr'), 'Aucun élément');
    expect(formatMessage(c.messages['itemCount']!, {'count': 2}, 'fr'), '2 éléments');

    cdn.calls.clear();
    expect(await c.refresh('fr-FR'), isFalse);
    expect(cdn.calls.map((c) => c.$1), ['https://glot.test/cdn/demo/locales.json', 'https://glot.test/cdn/demo/fr.json']);
    expect(cdn.calls.every((c) => c.$2 != null), isTrue);

    cdn.down = true;
    final again = core(dir, cdn)..loadCached('fr');
    expect(again.messages['home.title'], 'Bonjour');
    expect(await again.refresh('fr'), isFalse);
    expect(again.messages['home.title'], 'Bonjour');
  });

  test('new language without a store release', () async {
    final c = core(tmp(), Cdn({'locales.json': locales, 'pl.json': pl}));
    expect(await c.refresh('pl-PL'), isTrue);
    expect(c.locale, 'pl');
    expect(formatMessage(c.messages['itemCount']!, {'count': 3}, 'pl'), '3 elementy');
    expect(formatMessage(c.messages['itemCount']!, {'count': 5}, 'pl'), '5 elementów');
  });

  test('broken CDN never throws and keeps nothing bad', () async {
    for (final cdn in [Cdn({}), Cdn({'locales.json': '{', 'fr.json': 'nope'}), Cdn({})..down = true]) {
      final c = core(tmp(), cdn);
      expect(await c.refresh('fr'), isFalse);
      expect(c.messages, isEmpty);
    }
    expect(GlotCore.pick('../x', ['en']), 'en');
  });
}
