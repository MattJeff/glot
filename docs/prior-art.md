# Prior art — production translation feedback

Background for a freedom-to-operate review of glot's feedback loop (`@glot-i18n/feedback`, `crates/server/src/feedback.rs`).
**This is not legal advice and draws no legal conclusion.** It lists public material for counsel to assess.
Links checked on **2026-10-18**. Dates are approximate where marked "~"; counsel should confirm them with archived copies (e.g. web.archive.org).

## Nearby patent

- **US 12,412,048 B2**, *Automatically integrating user translation feedback*, IBM. Filed 2021-12-21 (Appl. 17/557,737),
  published as US 2023/0196034 A1 (2023-06-22), granted 2025-09-09. CPC G06F 40/51.
  Gazette summary: a user flags a mistranslated word in an application interface; a glossary association relationship set
  (built by association analysis over clustered word resources) selects which *other* instances of that word to update;
  a new application version with revised translations is generated.
  - https://patentsgazette.uspto.gov/week36/OG/html/1538-2/US12412048-20250909.html
  - https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12412048
  - https://patents.justia.com/patent/20230196034

## What glot does (scope kept deliberately narrow)

1. An end user selects a rendered text (or clicks "Improve" on the runtime badge) and reports it as wrong / awkward / unclear, with an optional suggestion.
2. The server maps the text to **one** catalog key (exact match with placeholders as wildcards, else substring) and groups reports by (key, locale).
3. At a configurable report count, or on demand, the existing translation pipeline proposes a new value **for that one key** (validator + glossary check + retries), and the existing Review API back-translates it.
4. A human accepts (written as a human, reviewed translation) or rejects. Optionally a GitHub PR is opened for that key's catalog line and `glot.lock` entry.

Not implemented, on purpose: propagating a correction to other keys or other occurrences of a word, glossary association
analysis / clustering of word resources, automatic glossary updates from feedback, automatic verification of feedback
across languages, automatic publication without human acceptance. Changes in that direction should go through review first.

## Earlier public systems (in-context / user-sourced translation corrections)

| System | What is public | Approx. date | Links |
|---|---|---|---|
| Mozilla Pontoon | In-place localization: select a page element (even buttons) in the live site and replace its text; translations stored in VCS | 2012 announcement | https://blog.mozilla.org/l10n/2012/05/02/coming-soon-to-mozilla-web-l10n-wysiwyg-tool-pontoon/, https://github.com/mozilla/pontoon, https://pontoon.mozilla.org |
| Crowdin In-Context | One-line JS snippet + pseudo-language; translators/users suggest translations directly in the running app, usable on production for a chosen user segment; suggestions land in the project with voting/approval and glossary | ~2014– | https://support.crowdin.com/in-context-localization/, https://support.crowdin.com/enterprise/in-context, https://crowdin.com/page/in-context-localization, https://www.jetbrains.com/help/youtrack/cloud/in-context-localization.html (deployed example: edit icon on UI text → suggest translation) |
| Transifex Live | JS snippet on a website; translate and review in context; approve, publish | ~2014– | https://help.transifex.com/en/collections/3452915-transifex-live, https://wordpress.org/plugins/transifex-live-integration/ |
| Facebook "Translate Facebook" | Crowd-sourced UI translation with in-context suggestions and voting by users | ~2008 | to locate via archived press coverage |
| Google Translate "Contribute / Suggest an edit" | End users suggest a better translation; community verification | ~2014 (Translate Community) | https://translate.google.com/community (verify archived copies) |
| Microsoft Translator Collaborative Translation Framework | Users submit and rate corrections to machine translations (API `AddTranslation`) | ~2010 | to locate via archived Microsoft Translator blog/docs |

Related earlier patents surfaced by the same search (not analysed): US 7,983,897 B2 *Machine translation feedback*
(https://patents.google.com/patent/US7983897), US 8,996,352 / US 9,245,278 *Systems and methods for correcting
translations in multi-user multi-lingual communications*, US 11,429,795 B2 *Machine translation integrated with user analysis*
(https://patents.google.com/patent/US11429795).
