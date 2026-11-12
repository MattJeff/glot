# Text graph ("universal anchor")

Status: first version built (server module, API, dashboard page, docs, tests). Labs.

## Problem

Every localization tool manages files of keys, one project at a time. A company does not own keys, it owns
sentences, and the same sentence lives in its code catalog, on the pages of its site, in its CMS, in its emails.
Today each copy is translated, reviewed and fixed separately. The same sentence ends up translated two ways, a
fix made on the live site never reaches the catalog, and a key rename loses the translation.

## What exists on the market

- Lokalise: key referencing links keys by id and a duplicate finder links duplicates, inside one project and
  between keys only. https://docs.lokalise.com/developer-docs/key-referencing
- Crowdin: duplicate strings can be hidden or auto-translated from a "master" string; sharing only fills
  untranslated duplicates, between files of the same format. https://support.crowdin.com/project-settings/import/
- Smartling: string uniqueness by text + variant + namespace, SmartMatch applies exact TM matches.
  https://help.smartling.com/hc/en-us/articles/115003184894
- Localazy: duplicity linking. https://localazy.com/features/duplicity-linking
- WebTranslateIt: TM matching across placeholder formats.
  https://webtranslateit.com/blog/posts/442-smarter-translation-memory-matching-across-placeholder-formats
- Ditto, Frontitude: one source of truth for product copy between Figma and code, as reusable text components
  a team wires by hand. https://www.dittowords.com/docs/what-is-ditto

All of them deduplicate *strings in files of keys* or reuse past translations through a translation memory.

## What is new, honestly

Not new: exact-duplicate detection, TM reuse, placeholder-format matching, "reviewed beats machine".

New in combination, and not found elsewhere:

1. The node is the sentence and the places are edges, across stores of different natures: catalog keys, text
   observed on rendered pages (glot Edge / website tag), CMS fields, and any imported surface. Other tools only
   see what was uploaded as a file.
2. Identity is derived from the text (deterministic hash), so nothing has to be linked by hand and a key rename
   cannot break it. Lokalise links ids; rename-safe but manual and key-to-key.
3. Translations are not copied into the graph: they are read live from each store and written back through each
   store's own validation. There is no second source of truth to drift.
4. Inconsistency (two machine translations) and conflict (two reviewed translations) are separate states, and
   automatic propagation refuses both.

glot already had, before this work: the Edge TM falling back to the catalog translation of the exact same text
(`edge::tm::snapshot`), connector content stored as catalog keys, a per-key translation history. The graph uses
all three and does not replace them.

## Architecture

One module, `crates/server/src/text_graph.rs`. No change to the code of the catalog, Edge or connectors.

Identity of a text (pure functions, unit-tested):

| Level | Ignores | Used for |
| --- | --- | --- |
| exact | whitespace runs | equals glot Edge's segment hash; tells when Edge already serves a catalog key |
| norm | + placeholder syntax, quote style, NBSP, `…` | the anchor (with context) |
| loose | + case, punctuation | "same words" related sentence, never merged |
| trigram similarity ≥ 0.8 | small wording changes (texts of 16+ chars) | "nearly the same" related sentence, never merged |

Context: `action` for a text of at most three words on a button or link, an explicit importer context, else
none. Anchor id = hash(source locale, normalized text, context): deterministic, no registry.

Sync (`sync`) mirrors *places only*, read-only on `keys`, `connector_items`, `edge_segments`, in one
transaction under a per-project advisory lock. Reads sync when the graph is older than 60 s; writes always
sync first.

Propagation (`analyze` + `plan`, pure): per sentence and locale, group the live translations by comparable
form, pick a winner only when it is undisputed, then decide per place: write, or skip with a reason
(`reviewed_conflict`, `machine_disagrees`, `needs_review`, `placeholders`, `validation`). `apply` writes
through `keys::write_translation` (catalog and connector keys), an upsert on `edge_translations` (same shape
as Edge's own human edit), or `text_translations` (imported surfaces).

## Data model (migration 0177)

- `text_anchors(project_id, id, norm_hash, loose_hash, context, source_text)`
- `text_occurrences(project_id, surface, locator, anchor_id, exact_hash, source_text, role, meta,
  prev_anchor_id, moved_at, seen_at)`; `prev_anchor_id` records a rewording.
- `text_translations(project_id, surface, locator, locale, value, status, origin)`: imported surfaces only.
- `text_graph_state(project_id, synced_at)`.

Migrations 0178 and 0179 are unused.

## Privacy and security

- The graph holds text the project already stores. Building it calls no LLM and no third party.
- No visitor data. Website places carry a path without query string and a hit counter, both already stored by
  glot Edge.
- All routes go through `projects::load`: Translator to read, unify and propagate (same right as editing a
  translation), Admin to import. No URL is fetched server-side; an importer `url` is only displayed, and the
  dashboard links it only when it is http(s).
- Import is bounded (5000 items, 20 kB per text) and validated item by item.

## Pricing idea

Counting is by sentence, not by key or word: a sentence shown in five places is translated and reviewed once.
Free and Starter: the graph over catalog + website. Pro: connectors in the graph and propagation. Team:
imported surfaces (emails, design, documents) and the API. Not implemented: nothing is gated today.

## Phases

1. Done: identity, graph, sync of the three stores, propagation rules, unify, importer contract, dashboard.
2. Next: call `sync` + `propagate` after a translation job and after an Edge review (one line each, see
   below), so that "apply everywhere" needs no click. Search by translation text.
3. Semantic links: embeddings behind a provider trait to relate paraphrases. Needs an embedding API key and a
   cost decision; suggestions only, never automatic propagation.
4. Performance per sentence: join Edge hits (already shown) with insights and experiment results per anchor.
5. Importers: email templates, Figma. The contract exists; no importer is written.

Hooks other owners can add (not added: their files are not mine):
- end of the translate job in `jobs.rs`: `crate::text_graph::sync(&state.db, &p).await` then the body of
  `propagate`;
- `connectors::sync`: nothing needed, connector keys are catalog keys and are pushed on the next sync.

## Risks

- Wrong merge of two meanings. Mitigation: only undisputed translations propagate, short action texts are
  split, every write is validated, catalog writes are in the key history and reversible there.
- A short text with an unknown role (a catalog key without `ui_role`) shares a sentence with plain-text uses.
  Honest limit; the inconsistency list is what catches it.
- Scale: sync and the project-wide views are full passes in memory (marked `ponytail:` with the upgrade path).
  Fine for tens of thousands of places; incremental sync is needed beyond ~200k.
- When two catalog keys with the same short text sit in different contexts, Edge's own fallback picks one of
  them by key order; the graph shows the Edge place as following the key of its own sentence.
