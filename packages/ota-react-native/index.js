// i18next backend: glot's CDN over the air, cached in AsyncStorage, on top of the JSON bundled in the app.
// Contract: https://www.i18next.com/misc/creating-own-plugins#backend
// i18next does plurals (`key_one`, `key_other`) and interpolation (`{{name}}`) itself: values are served as
// glot stores them for an i18next project. Nothing about the user is sent: two GETs (`locales.json`,
// `{locale}.json`) with `If-None-Match` and `credentials: "omit"`. Never throws, never rejects.

const SAFE = /^[A-Za-z0-9_-]{1,64}$/;

/** @param {string} s */
const norm = (s) => s.replace(/_/g, "-").toLowerCase();

/**
 * First ready language matching `wanted` (exact, then same language), else `wanted`'s language.
 * @param {string} wanted @param {string[]} available
 */
export function pick(wanted, available) {
  const base = (/** @type {string} */ s) => norm(s).split("-")[0];
  const hit = available.find((a) => norm(a) === norm(wanted)) ?? available.find((a) => base(a) === base(wanted));
  if (hit) return hit;
  return SAFE.test(wanted) ? base(wanted) : "en";
}

/** @param {any} target @param {any} src */
function merge(target, src) {
  const out = { ...target };
  for (const [k, v] of Object.entries(src ?? {})) {
    out[k] = v && typeof v === "object" && !Array.isArray(v) && out[k] && typeof out[k] === "object" ? merge(out[k], v) : v;
  }
  return out;
}

/** @param {string | null | undefined} text */
function parse(text) {
  try {
    const v = text == null ? null : JSON.parse(text);
    return v && typeof v === "object" && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

export class GlotOtaBackend {
  static type = /** @type {const} */ ("backend");
  type = /** @type {const} */ ("backend");

  /** @param {any} [services] @param {import("./index.js").GlotOtaOptions} [options] */
  constructor(services, options) {
    /** @type {any} */
    this.services = null;
    /** @type {import("./index.js").GlotOtaOptions} */
    this.o = /** @type {any} */ ({});
    /** @type {Promise<string[]> | null} */
    this._locales = null;
    if (services) this.init(services, options);
  }

  /** @param {any} services @param {import("./index.js").GlotOtaOptions} [options] */
  init(services, options) {
    this.services = services;
    this.o = options ?? /** @type {any} */ ({});
    this._locales = null;
  }

  /** Languages glot has ready (source first), cached; [] when unknown. Offer them in a picker. */
  locales() {
    const p = (this._locales ??= this._get("locales.json").then((r) => {
      const list = parse(r?.body)?.locales;
      if (!Array.isArray(list)) this._locales = null; // retry on the next read
      return Array.isArray(list)
        ? list.filter((e) => e && typeof e.code === "string" && SAFE.test(e.code) && e.ready !== false).map((e) => e.code)
        : [];
    }));
    return p;
  }

  /** @param {string} language @param {string} namespace @param {(err: unknown, data?: unknown) => void} cb */
  read(language, namespace, cb) {
    const task = async () => {
      const bundled = await this._bundled(language, namespace);
      const code = pick(language, await this._cachedLocales());
      const cached = await this._load(code);
      const pickNs = (/** @type {any} */ cat) => (cat && typeof cat[namespace] === "object" ? cat[namespace] : cat);
      const first = merge(bundled, pickNs(parse(cached?.body)));
      const update = this._refresh(language, cached?.body).then((body) => body && merge(bundled, pickNs(parse(body))));
      if (cached || Object.keys(bundled).length) {
        cb(null, first);
        const next = await update;
        if (next) {
          this.services?.resourceStore?.addResourceBundle(language, namespace, next, true, true);
          this.o.onUpdate?.(language);
        }
      } else {
        cb(null, (await update) ?? first); // first run of a language the app doesn't embed: wait for it
      }
    };
    task().catch(() => cb(null, {}));
  }

  /** New catalog body for `language` when the CDN has one different from `old`; null otherwise. */
  async _refresh(/** @type {string} */ language, /** @type {string | undefined} */ old) {
    const code = pick(language, await this.locales());
    const r = await this._get(`${code}.json`);
    return r && r.fresh && r.body !== old && parse(r.body) ? r.body : null;
  }

  async _cachedLocales() {
    const list = parse((await this._load("locales"))?.body)?.locales;
    return Array.isArray(list) ? list.filter((e) => e?.ready !== false && typeof e?.code === "string").map((e) => e.code) : [];
  }

  /** @param {string} lng @param {string} ns @returns {Promise<Record<string, unknown>>} */
  async _bundled(lng, ns) {
    try {
      const b = this.o.bundled;
      const v = typeof b === "function" ? await b(lng, ns) : (b?.[lng]?.[ns] ?? b?.[lng.split(/[-_]/)[0]]?.[ns]);
      return v && typeof v === "object" ? /** @type {any} */ (v) : {};
    } catch {
      return {};
    }
  }

  _key(/** @type {string} */ name) {
    return `glot:${this.o.project}:${name}`;
  }

  /** @param {string} name @returns {Promise<{ etag?: string, body: string } | null>} */
  async _load(name) {
    try {
      const v = parse(await this.o.storage?.getItem(this._key(name)));
      return v && typeof v.body === "string" ? /** @type {any} */ (v) : null;
    } catch {
      return null;
    }
  }

  /** GET with If-None-Match; 304 → the cached body. @param {string} file */
  async _get(file) {
    const name = file.replace(/\.json$/, "");
    if (!SAFE.test(this.o.project ?? "") || !SAFE.test(name)) return null;
    const cached = await this._load(name);
    try {
      const f = this.o.fetch ?? globalThis.fetch;
      const res = await f(`${String(this.o.cdn).replace(/\/+$/, "")}/cdn/${this.o.project}/${file}`, {
        headers: cached?.etag ? { "If-None-Match": cached.etag } : {},
        credentials: "omit",
      });
      if (res.status === 304) return cached ? { body: cached.body, fresh: false } : null;
      if (res.status !== 200) return null;
      const body = await res.text();
      if (!parse(body)) return null;
      const etag = res.headers.get("etag") ?? undefined;
      try {
        await this.o.storage?.setItem(this._key(name), JSON.stringify({ etag, body }));
      } catch {}
      return { body, fresh: true };
    } catch {
      return null;
    }
  }
}

export default GlotOtaBackend;
