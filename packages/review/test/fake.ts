// In-memory server following docs/design/live-review-contract.md (prefix W), plus a static demo page.
// `node test/fake.ts [port]` runs it standalone (demo: http://127.0.0.1:3501/?glot_review=glr_edit).
import { createServer, type ServerResponse } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { randomUUID } from "node:crypto";

const dist = join(import.meta.dirname, "../dist");
const enc = (key: string) => { let s = "⁣"; for (const b of new TextEncoder().encode(key)) s += String.fromCodePoint(b < 16 ? 0xfe00 + b : 0xe0100 + b - 16); return s + "⁣"; };

export function page(lang = "fr", dir = "ltr", wdir = "", appo = "", attrs = 'data-project="acme"') {
  return `<!doctype html><html lang="${lang}" dir="${dir}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Acme</title><style>
body{font:16px/1.5 Georgia,serif;margin:0;color:#222;background:#fbfaf7} header{display:flex;gap:16px;padding:16px 24px;background:#222;color:#fff}
header a{color:#fff} main{max-width:720px;margin:0 auto;padding:32px 24px} h1{font-size:40px;line-height:1.1}
/* hostile site CSS: must not leak into the widget */ button{background:hotpink!important;font-size:22px!important;border-radius:0!important} *{letter-spacing:.02em}
.card{border:1px solid #ddd;border-radius:12px;padding:16px;margin:16px 0;background:#fff}
</style></head><body>
<header><a href="/" id="logo">Acme</a><a href="/pricing">${lang === "ar" ? "الأسعار" : "Tarifs"}</a></header>
<main>
<h1 data-glot-key="home.title">${lang === "ar" ? "ترجم تطبيقك في 5 دقائق" : "Traduisez votre app en 5 minutes"}</h1>
<p data-testid="lead">${enc("home.lead")}Votre produit parle déjà toutes les langues de vos clients.</p>
<div class="card"><p>${enc("home.inbox")}Vous avez 3 messages</p><button id="cta">Commencer l'essai gratuit</button></div>
<p id="legal">Texte en dur sans clé.</p>
${Array.from({ length: 12 }, (_, i) => `<p>Paragraphe de remplissage ${i + 1} pour tester le défilement des épingles.</p>`).join("")}
</main>
<script src="/review.global.js" ${attrs} data-app-url="https://app.glot.test"${appo ? ` data-app-origin="${appo}"` : ""}${wdir ? ` data-dir="${wdir}"` : ""}></script>
</body></html>`;
}

type Tr = { value: string; status: string; issues: any[] };
export function serve(port = 3501) {
  const actors: Record<string, any> = {
    glr_edit: { access: "edit", me: { user_id: null, link_id: "L1", name: "Ana (lien)" } },
    glr_member: { access: "member", me: { user_id: "U1", link_id: null, name: "Mathis" } },
    glr_comment: { access: "comment", me: { user_id: null, link_id: "L2", name: "" } },
    glr_read: { access: "read", me: { user_id: null, link_id: "L3", name: "Lecteur" } },
  };
  const anns: any[] = [];
  const shots = new Map<string, Buffer>();
  const log: { method: string; path: string; body: any; token: string }[] = [];
  const clients = new Set<ServerResponse>();
  let evId = 0;
  const texts: Record<string, { source: string; values: Record<string, Tr>; hist: Record<string, string[]> }> = {
    "home.title": { source: "Translate your app in 5 minutes", values: { fr: { value: "Traduisez votre app en 5 minutes", status: "reviewed", issues: [] }, de: { value: "Übersetzen Sie Ihre App in 5 Minuten", status: "machine", issues: [] } }, hist: {} },
    "home.lead": { source: "Your product already speaks your customers' languages.", values: { fr: { value: "Votre produit parle déjà toutes les langues de vos clients.", status: "machine", issues: [] } }, hist: {} },
    "home.inbox": { source: "You have {count} messages", values: { fr: { value: "Vous avez {count} messages", status: "machine", issues: [] } }, hist: {} },
  };
  const targets = ["fr", "de", "ar"];
  const validate = (key: string, v: string) => {
    const src = texts[key].source, ph = (s: string) => (s.match(/\{\w+/g) || []).sort().join();
    const is: any[] = [];
    if (ph(src) !== ph(v)) is.push({ code: "placeholder_mismatch", severity: "error", message: `Placeholders differ from the source: expected ${src.match(/\{\w+\}/g)?.join(" ") || "none"}` });
    if (v.length > 80) is.push({ code: "too_long", severity: "warning", message: "Much longer than the source" });
    return is;
  };
  const entry = (key: string) => ({ key, source: texts[key].source, source_locale: "en", description: null, ui_role: null,
    values: Object.fromEntries(targets.map((l) => [l, texts[key].values[l] || { value: "", status: "missing", issues: [] }])) });
  const emit = (kind: string, actor: any, annotation_id: string | null, data: object = {}) => {
    const ev = { id: ++evId, annotation_id, comment_id: null, kind, actor, data, created_at: new Date().toISOString() };
    for (const c of clients) c.write(`event: review\nid: ${ev.id}\ndata: ${JSON.stringify(ev)}\n\n`);
  };
  const route = (url: string, locale: string) => { let p = new URL(url).pathname.replace(/\/+$/, "") || "/"; const s = p.split("/")[1]; if (s === locale) p = p.slice(s.length + 1) || "/"; return p; };
  const full = (a: any, withComments = false) => ({ ...a, comment_count: a.comments.length, approvals: a.reactions.find((r: any) => r.emoji === "approve")?.count || 0,
    reactions: a.reactions.map((r: any) => ({ emoji: r.emoji, count: r.actors.length, mine: false, actors: r.actors })), comments: withComments ? a.comments : undefined,
    translation: a.key && texts[a.key]?.values[a.locale] ? { value: texts[a.key].values[a.locale].value, status: texts[a.key].values[a.locale].status } : null });
  const toggle = (list: any[], emoji: string, who: string) => {
    let r = list.find((x) => x.emoji === emoji);
    if (!r) list.push((r = { emoji, actors: [] }));
    r.actors = r.actors.includes(who) ? r.actors.filter((x: string) => x !== who) : [...r.actors, who];
  };
  const mineR = (rs: any[], who: string) => rs.map((r) => ({ emoji: r.emoji, count: r.actors?.length ?? r.count, mine: (r.actors || []).includes(who) }));

  const server = createServer(async (req, res) => {
    const u = new URL(req.url!, "http://x");
    const send = (code: number, body?: unknown, type = "application/json") =>
      res.writeHead(code, { "content-type": type }).end(body === undefined ? undefined : typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body));
    const fail = (code: number, c: string, message: string, details?: unknown) => send(code, { error: { code: c, message, details } });
    if (!u.pathname.startsWith("/api/")) {
      if (u.pathname.endsWith(".js")) { try { return send(200, readFileSync(join(dist, u.pathname)), "text/javascript"); } catch { return send(404, ""); } }
      // Stand-in for the glot Review page (RV3): records the widget's messages, posts ours.
      if (u.pathname === "/parent") return send(200, `<!doctype html><body style="margin:0"><iframe id="f" src="${u.searchParams.get("src")}" style="width:1000px;height:640px;border:0"></iframe>
<script>window.msgs=[];addEventListener("message",(e)=>msgs.push({origin:e.origin,...e.data}));window.send=(m)=>f.contentWindow.postMessage(m,"*");</script></body>`, "text/html");
      return send(200, page(u.searchParams.get("lang") || "fr", u.searchParams.get("dir") || "ltr", u.searchParams.get("wdir") || "", u.searchParams.get("appo") || ""), "text/html");
    }
    const token = (req.headers.authorization || "").replace("Bearer ", "") || u.searchParams.get("token") || "";
    const who = actors[token];
    if (!who) return fail(401, "invalid_review_token", "Review link expired or revoked");
    const raw: Buffer[] = [];
    for await (const c of req) raw.push(c as Buffer);
    const buf = Buffer.concat(raw);
    const body = req.headers["content-type"]?.includes("json") && buf.length ? JSON.parse(buf.toString()) : {};
    const p = u.pathname.replace(/^\/api\/review\/w/, "");
    log.push({ method: req.method!, path: p + u.search, body: req.headers["content-type"]?.startsWith("image/") ? `<${buf.length} B image>` : body, token });
    const m = req.method!;
    const actor = { ...who.me, name: who.me.user_id ? who.me.name : body.author_name || u.searchParams.get("author_name") || who.me.name };
    const write = m !== "GET";
    if (write && who.access === "read") return fail(403, "forbidden", "Read-only link");
    const needsName = write && !who.me.user_id && !actor.name;
    if (needsName && !p.includes("/attachments")) return fail(400, "author_name_required", "author_name is required");
    let x: RegExpExecArray | null;

    if (p === "/context") return send(200, { project: { slug: "acme", name: "Acme", source_locale: "en", target_locales: targets }, access: who.access, me: who.me, expires_at: null });
    if (p === "/events") {
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
      res.write(": hi\n\n");
      clients.add(res);
      req.on("close", () => clients.delete(res));
      return;
    }
    if (p === "/annotations" && m === "GET") {
      const st = (u.searchParams.get("status") || "open,in_progress").split(",");
      const items = anns.filter((a) => a.route === u.searchParams.get("route") && a.locale === u.searchParams.get("locale") && st.includes(a.status))
        .map((a) => ({ ...full(a), reactions: mineR(a.reactions, actor.link_id || actor.user_id) }));
      return send(200, { items, total: items.length });
    }
    if (p === "/annotations" && m === "POST") {
      const an = body.anchor;
      const key = an.key && texts[an.key] ? an.key : Object.keys(texts).find((k) => texts[k].values[an.locale]?.value === an.text) || null;
      const a = { id: randomUUID(), status: "open", anchor: { ...an, url: an.url.split(/[?#]/)[0] }, route: route(an.url, an.locale), locale: an.locale, url: an.url, key,
        source: key ? texts[key].source : null, body: body.body, body_html: body.body.replace(/[<&]/g, (c: string) => (c === "<" ? "&lt;" : "&amp;")), suggestion: body.suggestion ?? null,
        author: actor, assignee: null, feedback_group_id: null, attachments: [] as any[], reactions: [] as any[], comments: [] as any[],
        created_at: new Date().toISOString(), updated_at: new Date().toISOString(), resolved_at: null };
      anns.push(a);
      emit("annotation.created", actor, a.id);
      return send(201, full(a));
    }
    if ((x = /^\/annotations\/([\w-]+)(\/.*)?$/.exec(p))) {
      const a = anns.find((y) => y.id === x![1]);
      if (!a) return fail(404, "not_found", "Annotation not found");
      const sub = x[2] || "";
      const own = (o: any) => (o.author.user_id || o.author.link_id) === (actor.user_id || actor.link_id);
      if (!sub && m === "GET") return send(200, { ...full(a, true), reactions: mineR(a.reactions, actor.link_id || actor.user_id), comments: a.comments.map((c: any) => ({ ...c, reactions: mineR(c.reactions, actor.link_id || actor.user_id) })) });
      if (!sub && m === "PATCH") {
        if (body.status && who.access === "comment") return fail(403, "forbidden", "Changing the status needs an edit link");
        Object.assign(a, body, { updated_at: new Date().toISOString() });
        if (body.body) a.body_html = body.body;
        emit("annotation.updated", actor, a.id, body);
        return send(200, full(a));
      }
      if (!sub && m === "DELETE") { if (!own(a)) return fail(403, "forbidden", "Not yours"); anns.splice(anns.indexOf(a), 1); emit("annotation.deleted", actor, a.id); return send(204); }
      if (sub === "/comments" && m === "POST") {
        const c = { id: randomUUID(), annotation_id: a.id, author: actor, body: body.body, body_html: body.body, mentions: [], reactions: [], created_at: new Date().toISOString(), edited_at: null };
        a.comments.push(c); emit("comment.created", actor, a.id); return send(201, c);
      }
      if ((x = /^\/comments\/([\w-]+)$/.exec(sub))) {
        const c = a.comments.find((y: any) => y.id === x![1]);
        if (m === "PATCH") { c.body = c.body_html = body.body; c.edited_at = new Date().toISOString(); emit("comment.updated", actor, a.id); return send(200, c); }
        a.comments.splice(a.comments.indexOf(c), 1); emit("comment.deleted", actor, a.id); return send(204);
      }
      if (sub === "/reactions") {
        const tgt = body.comment_id ? a.comments.find((c: any) => c.id === body.comment_id) : a;
        toggle(tgt.reactions, body.emoji, actor.link_id || actor.user_id);
        emit("reaction.toggled", actor, a.id, { emoji: body.emoji });
        return send(200, { reactions: mineR(tgt.reactions, actor.link_id || actor.user_id) });
      }
      if (sub === "/attachments" && m === "POST") {
        const id = randomUUID(); shots.set(id, buf);
        a.attachments.push({ id, content_type: req.headers["content-type"], size: buf.length, width: null, height: null, created_at: new Date().toISOString() });
        emit("attachment.created", actor, a.id); return send(201, a.attachments.at(-1));
      }
      if ((x = /^\/attachments\/([\w-]+)$/.exec(sub))) return send(200, shots.get(x[1]), "image/png");
      if (sub === "/apply") {
        if (!a.key) return fail(409, "no_key", "No key");
        const v = body.value ?? a.suggestion, is = validate(a.key, v);
        if (is.some((i) => i.severity === "error")) return fail(422, "invalid_translation", is[0].message);
        (texts[a.key].hist[a.locale] ||= []).push(texts[a.key].values[a.locale]?.value);
        texts[a.key].values[a.locale] = { value: v, status: "reviewed", issues: is };
        a.status = "resolved"; emit("suggestion.applied", actor, a.id, { key: a.key, locale: a.locale, value: v });
        return send(200, full(a));
      }
      if (sub === "/rephrase") {
        // RV4 shape: proposals declined in every locale, best valid one becomes the suggestion.
        const base = a.anchor.text as string, words = base.split(" ");
        const texts2 = body.preset === "shorter" ? [words.slice(0, Math.ceil(words.length / 2)).join(" "), words.slice(0, 3).join(" ")] : [base + " !", base.toUpperCase()];
        const proposals = texts2.map((t, id) => ({ id, text: t, explanation: id ? "Encore plus court" : "Moitié moins long",
          variants: Object.fromEntries(["en", ...targets].map((l) => [l, { value: l === a.locale ? t : `[${l}] ${t}`, back_translation: null, issues: id ? [{ code: "too_short", severity: "warning", message: "Peut-être trop court" }] : [], policy: [], valid: true }])), valid: true }));
        a.suggestion = proposals[0].text; a.rewrite = proposals; emit("suggestion_set", actor, a.id, { suggestion: a.suggestion });
        return send(200, { rewrite_id: "rw-" + a.id, cached: false, created_at: new Date().toISOString(), key: a.key, locale: a.locale, proposals, rejected: [], model: "fake" });
      }
      if (sub === "/rephrase/apply") {
        if (who.access === "comment") return fail(403, "forbidden", "Needs an edit link");
        const pr = a.rewrite[body.proposal_id];
        for (const [l, v] of Object.entries(pr.variants) as [string, any][]) if (l !== "en" && a.key) texts[a.key].values[l] = { value: v.value, status: "reviewed", issues: [] };
        a.status = "resolved"; a.suggestion = pr.text; emit("suggestion.applied", actor, a.id, {});
        return send(200, { annotation: full(a), results: {}, source_text: null });
      }
    }
    if ((x = /^\/text\/([^/]+)(\/.*)?$/.exec(p))) {
      const key = decodeURIComponent(x[1]), sub = x[2] || "";
      if (!texts[key]) return fail(404, "not_found", "Unknown key");
      if (write && who.access !== "edit" && who.access !== "member") return fail(403, "forbidden", "Needs an edit link");
      const t = texts[key];
      if (!sub && m === "GET") return send(200, entry(key));
      if (!sub && m === "PUT") {
        const results = Object.fromEntries(Object.entries(body.values as Record<string, string>).map(([l, v]) => [l, { issues: validate(key, v), translation: null }]));
        if (!body.dry_run) {
          if (Object.values(results).some((r) => r.issues.some((i: any) => i.severity === "error"))) return fail(422, "invalid_translation", "Invalid translation", { results });
          for (const [l, v] of Object.entries(body.values as Record<string, string>)) { (t.hist[l] ||= []).push(t.values[l]?.value); t.values[l] = { value: v, status: "reviewed", issues: [] }; }
          emit("text.updated", actor, null, { key, locales: Object.keys(body.values) });
        }
        return send(200, { results });
      }
      if (sub === "/revert") {
        const prev = t.hist[body.locale]?.pop();
        if (prev === undefined) return fail(409, "nothing_to_revert", "Nothing to revert");
        t.values[body.locale] = { value: prev, status: "reviewed", issues: [] };
        emit("text.reverted", actor, null, { key, locale: body.locale });
        return send(200, { translation: t.values[body.locale] });
      }
      if ((x = /^\/translations\/([\w-]+)$/.exec(sub))) {
        (t.hist[x[1]] ||= []).push(t.values[x[1]]?.value); delete t.values[x[1]];
        emit("text.deleted", actor, null, { key, locale: x[1] }); return send(204);
      }
    }
    if (p === "/text" && m === "POST") {
      const key = body.key || "home." + body.source.toLowerCase().replace(/[^a-z]+/g, "_").replace(/^_|_$/g, "").slice(0, 30);
      if (texts[key]) return fail(409, "key_exists", "Key exists");
      texts[key] = { source: body.source, values: {}, hist: {} };
      emit("text.created", actor, null, { key });
      return send(201, entry(key));
    }
    fail(404, "not_found", `${m} ${p}`);
  });
  return new Promise<{ close: () => void; log: typeof log; anns: any[]; texts: typeof texts; base: string }>((ok) =>
    server.listen(port, "127.0.0.1", () => ok({ close: () => { for (const c of clients) c.end(); server.close(); }, log, anns, texts, base: `http://127.0.0.1:${port}` })));
}

if (process.argv[1] === import.meta.filename) serve(Number(process.argv[2]) || 3501).then((s) => console.log(`fake glot on ${s.base}/?glot_review=glr_edit`));
