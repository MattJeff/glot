import assert from "node:assert/strict";
import { test } from "node:test";
import { GlotOtaBackend, pick } from "../index.js";

const locales = JSON.stringify({
  source: "en",
  locales: [
    { code: "en", ready: true },
    { code: "fr", ready: true },
    { code: "pl", ready: true },
    { code: "de", ready: false },
  ],
});
const fr = JSON.stringify({ home: { title: "Bonjour" }, items_one: "{{count}} élément", items_other: "{{count}} éléments" });

/** Fake CDN with ETags / 304s; records [url, If-None-Match, credentials]. */
function cdn(files) {
  const calls = [];
  const f = async (url, init) => {
    calls.push([url, init.headers["If-None-Match"], init.credentials, Object.keys(init.headers)]);
    if (f.down) throw new TypeError("Network request failed");
    const body = f.files[url.split("/").pop()];
    if (body == null) return new Response("not found", { status: 404 });
    const tag = `"${body.length}-${[...body].reduce((h, c) => (h * 31 + c.charCodeAt(0)) | 0, 0)}"`;
    if (init.headers["If-None-Match"] === tag) return new Response(null, { status: 304, headers: { etag: tag } });
    return new Response(body, { status: 200, headers: { etag: tag } });
  };
  f.files = files;
  f.calls = calls;
  f.down = false;
  return f;
}

const memory = () => {
  const m = new Map();
  return { m, getItem: async (k) => m.get(k) ?? null, setItem: async (k, v) => void m.set(k, v) };
};

/** i18next-like services: records addResourceBundle calls. */
const services = () => {
  const added = [];
  return { added, resourceStore: { addResourceBundle: (...a) => added.push(a) } };
};

const read = (b, lng, ns = "translation") => new Promise((res) => b.read(lng, ns, (err, data) => res({ err, data })));
const tick = () => new Promise((r) => setTimeout(r, 20));

test("first run waits for the CDN, then caches and revalidates", async () => {
  const storage = memory();
  const fetch = cdn({ "locales.json": locales, "fr.json": fr });
  const b = new GlotOtaBackend(services(), { project: "demo", cdn: "https://glot.test/", storage, fetch });
  const { err, data } = await read(b, "fr-FR");
  assert.equal(err, null);
  assert.equal(data.home.title, "Bonjour");
  assert.deepEqual(await b.locales(), ["en", "fr", "pl"]);
  assert.deepEqual(
    fetch.calls.map((c) => c[0]),
    ["https://glot.test/cdn/demo/locales.json", "https://glot.test/cdn/demo/fr.json"],
  );
  assert.ok(fetch.calls.every((c) => c[2] === "omit" && c[3].every((h) => h === "If-None-Match")));

  // Restart: cached strings right away, ETag sent, 304, nothing re-added.
  fetch.calls.length = 0;
  const s = services();
  const again = new GlotOtaBackend(s, { project: "demo", cdn: "https://glot.test", storage, fetch });
  assert.equal((await read(again, "fr-FR")).data.items_other, "{{count}} éléments");
  await tick();
  assert.ok(fetch.calls.every((c) => c[1]));
  assert.equal(s.added.length, 0);
});

test("bundled strings first, newer CDN strings added live", async () => {
  const fetch = cdn({ "locales.json": locales, "fr.json": fr });
  const s = services();
  let updated = null;
  const b = new GlotOtaBackend(s, {
    project: "demo",
    cdn: "https://glot.test",
    storage: memory(),
    fetch,
    bundled: { fr: { translation: { home: { title: "Bonjour (app)", sub: "Embarqué" } } } },
    onUpdate: (l) => (updated = l),
  });
  const { data } = await read(b, "fr");
  assert.equal(data.home.title, "Bonjour (app)"); // no wait on the network
  await tick();
  assert.equal(updated, "fr");
  const [lng, ns, next, deep, overwrite] = s.added[0];
  assert.deepEqual([lng, ns, deep, overwrite], ["fr", "translation", true, true]);
  assert.equal(next.home.title, "Bonjour");
  assert.equal(next.home.sub, "Embarqué"); // keys glot lacks keep the bundled value
});

test("offline, CDN errors and garbage never throw", async () => {
  for (const fetch of [cdn({}), cdn({ "locales.json": "{", "fr.json": "nope" }), Object.assign(cdn({}), { down: true })]) {
    const b = new GlotOtaBackend(services(), {
      project: "demo",
      cdn: "https://glot.test",
      storage: { getItem: () => { throw new Error("disk"); }, setItem: () => { throw new Error("disk"); } },
      fetch,
      bundled: (lng) => ({ hello: `bundled ${lng}` }),
    });
    const { err, data } = await read(b, "fr");
    assert.equal(err, null);
    assert.equal(data.hello, "bundled fr");
  }
  const none = new GlotOtaBackend(services(), { project: "../x", cdn: "https://glot.test", fetch: cdn({}) });
  assert.deepEqual((await read(none, "fr")).data, {});
});

test("new language without a store release", async () => {
  const pl = JSON.stringify({ items_few: "{{count}} elementy", items_many: "{{count}} elementów" });
  const b = new GlotOtaBackend(services(), {
    project: "demo",
    cdn: "https://glot.test",
    storage: memory(),
    fetch: cdn({ "locales.json": locales, "pl.json": pl }),
  });
  assert.equal((await read(b, "pl-PL")).data.items_few, "{{count}} elementy");
  assert.equal(pick("fr-CA", ["en", "fr"]), "fr");
  assert.equal(pick("de-DE", ["en", "fr"]), "de");
});
