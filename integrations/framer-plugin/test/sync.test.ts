// sync() against a Framer double (shapes from framer-api 5.1 / framer-plugin 3.10 `LocalizationGroup`,
// `LocalizationSource`, `setLocalizationData`) and a simulated glot server (sync/push, sync/pull, jobs).
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { glotClient, hash, keyOf, matchLocale, sync, type FramerGroup, type FramerLike } from "../src/sync.ts";

test("hash matches glot_core::hash::hash", async () => {
  assert.equal(await hash("test"), "9f86d081884c7d65");
});

test("locale matching", () => {
  assert.equal(matchLocale("fr-FR", ["de", "fr"]), "fr");
  assert.equal(matchLocale("pt-BR", ["pt-br"]), "pt-br");
  assert.equal(matchLocale("nl", ["de"]), undefined);
});

async function glotServer(project: string) {
  const state = {
    source: {} as Record<string, string>,
    notes: {} as Record<string, string>,
    fr: {} as Record<string, { value: string; source_hash: string; reviewed: boolean }>,
    jobs: [] as unknown[],
    auth: [] as string[],
  };
  const server = createServer(async (req, res) => {
    state.auth.push(req.headers.authorization ?? "");
    let body = "";
    for await (const c of req) body += c;
    const send = (code: number, v: unknown) => res.writeHead(code, { "content-type": "application/json" }).end(JSON.stringify(v));
    const p = `/api/plugin/projects/${project}`;
    if (req.method === "POST" && req.url === `${p}/sync/push`) {
      const b = JSON.parse(body);
      assert.deepEqual(Object.keys(b.catalogs), ["en"]);
      let created = 0;
      for (const [k, v] of Object.entries<string>(b.catalogs.en)) {
        if (!(k in state.source)) created++;
        state.source[k] = v;
        state.notes[k] = b.context[k].description;
      }
      return send(200, { created, updated: 0, conflicts: [], syntax: "icu" });
    }
    if (req.method === "GET" && req.url === `${p}/sync/pull`) {
      const fr = Object.fromEntries(Object.entries(state.fr).map(([k, t]) => [k, t.value]));
      const entries = {
        fr: Object.fromEntries(
          Object.entries(state.fr).map(([k, t]) => [k, { source_hash: t.source_hash, target_hash: "", origin: { kind: "machine", model: "m" }, reviewed: t.reviewed }]),
        ),
      };
      return send(200, { source_locale: "en", syntax: "icu", catalogs: { en: state.source, fr }, lock: { version: 1, notes: {}, entries }, glossary: [] });
    }
    if (req.method === "POST" && req.url === `${p}/jobs`) {
      const b = JSON.parse(body);
      state.jobs.push(b);
      // The simulated translator: "FR " + source, for every key without an up-to-date translation.
      for (const k of b.keys) {
        const h = await hash(state.source[k]);
        if (state.fr[k]?.source_hash !== h) state.fr[k] = { value: `FR ${state.source[k]}`, source_hash: h, reviewed: false };
      }
      return send(201, { id: "j1", status: "queued" });
    }
    if (req.method === "GET" && req.url === "/api/plugin/jobs/j1") return send(200, { id: "j1", status: "done" });
    send(404, { error: { message: "not found" } });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return { state, server, base: `http://127.0.0.1:${(server.address() as AddressInfo).port}` };
}

function fakeFramer(groups: FramerGroup[]) {
  const writes: unknown[] = [];
  const f: FramerLike = {
    getLocales: async () => [
      { id: "L-en", code: "en" },
      { id: "L-fr", code: "fr-FR" },
      { id: "L-nl", code: "nl" },
    ],
    getDefaultLocale: async () => ({ id: "L-en", code: "en" }),
    getLocalizationGroups: async () => groups,
    setLocalizationData: async (d) => {
      writes.push(d);
      for (const [sid, byLocale] of Object.entries(d.valuesBySource))
        for (const g of groups)
          for (const s of g.sources)
            if (s.id === sid)
              for (const [lid, u] of Object.entries(byLocale)) s.valueByLocale[lid] = { status: u.needsReview ? "needsReview" : "done", value: u.value };
      return { valuesBySource: { errors: [] }, statusByLocaleByGroup: { errors: [] } };
    },
  };
  return { f, writes };
}

test("push sources, translate, write back only valid translations, idempotent, keeps human values", async () => {
  const { state, server, base } = await glotServer("site");
  try {
    const groups: FramerGroup[] = [
      {
        id: "page1",
        name: "Home",
        type: "page",
        sources: [
          { id: "s1", type: "string", value: "Hello", valueByLocale: {} },
          { id: "s2", type: "formattedText", value: "<p>Read <a href=\"/t\">terms</a></p>", valueByLocale: {} },
          { id: "s3", type: "slug", value: "home", valueByLocale: {} },
          { id: "s4", type: "link", value: "https://x.test", valueByLocale: {} },
          { id: "s5", type: "string", value: "Kept", valueByLocale: { "L-fr": { status: "done", value: "Gardé (humain)" } } },
          { id: "s6", type: "string", value: "  ", valueByLocale: {} },
        ],
      },
    ];
    const { f, writes } = fakeFramer(groups);
    const glot = glotClient({ base, token: "glot_t", project: "site", prefix: "/api/plugin" });

    const r = await sync(f, glot);
    assert.equal(r.sources, 3, "slug, link and blank sources are not sent");
    assert.equal(r.created, 3);
    assert.deepEqual(r.unmatchedLocales, ["nl"]);
    assert.ok(state.auth.every((a) => a === "Bearer glot_t"));
    const k2 = keyOf(groups[0], groups[0].sources[1]);
    assert.equal(k2, "framer:page:page1:s2");
    assert.match(state.notes[k2], /Framer page “Home”.*keep every tag/);
    assert.deepEqual(state.jobs[0], { kind: "translate", keys: Object.keys(state.source), locales: ["fr"] });
    assert.equal(r.written, 2);
    assert.equal(r.skipped, 1, "the human value of s5 is kept");
    assert.deepEqual(writes[0], {
      valuesBySource: {
        s1: { "L-fr": { action: "set", value: "FR Hello", needsReview: true } },
        s2: { "L-fr": { action: "set", value: "FR <p>Read <a href=\"/t\">terms</a></p>", needsReview: true } },
      },
    });

    // Second run: nothing changed -> nothing written.
    const again = await sync(f, glot);
    assert.equal(again.written, 0);
    assert.equal(writes.length, 1);

    // Reviewed in glot -> the needsReview flag is cleared in Framer.
    state.fr[keyOf(groups[0], groups[0].sources[0])].reviewed = true;
    const reviewed = await sync(f, glot, { translate: false });
    assert.equal(reviewed.written, 1);
    assert.deepEqual(writes[1], { valuesBySource: { s1: { "L-fr": { action: "set", value: "FR Hello", needsReview: false } } } });

    // Source edited in Framer, no translation run: the stale translation is not written.
    groups[0].sources[0].value = "Hello there";
    const stale = await sync(f, glot, { translate: false });
    assert.equal(stale.written, 0);
    assert.ok(stale.skipped >= 1);

    // --force overwrites the human value.
    const forced = await sync(f, glot, { force: true });
    assert.equal(groups[0].sources[4].valueByLocale["L-fr"].value, "FR Kept");
    assert.ok(forced.written >= 2);
  } finally {
    server.close();
  }
});

test("glot errors surface with the server message", async () => {
  const { server, base } = await glotServer("site");
  try {
    const glot = glotClient({ base, token: "t", project: "nope", prefix: "/api/plugin" });
    await assert.rejects(glot.pull(), /HTTP 404 not found/);
  } finally {
    server.close();
  }
});
