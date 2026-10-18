// OpenAI-compatible endpoint for the refusal demo (`refusal.sh`).
// First pass: answers like a sloppy LLM on purpose (Polish plural without `few`/`many`, a translated
// `{name}`, a German label too wide for its button). Retries (prompt carries `rejected_attempt` +
// `issues_to_fix`, written by glot's validator): the fixed wording, or the real model with UPSTREAM.
//   node refusal-llm.mjs 18799                                   scripted, offline
//   UPSTREAM=https://api.anthropic.com/v1 UPSTREAM_KEY=$ANTHROPIC_API_KEY node refusal-llm.mjs 18799
//     -> first pass still injected (a good model rarely fails on cue), every retry is the real model.
import http from "node:http";

const BAD = {
  "pl:inbox.unread": "{count, plural, one {# nieprzeczytana wiadomość} other {# nieprzeczytanych wiadomości}}",
  "fr:home.welcome": "Bon retour, {nom} !",
  "de:invoice.remind": "Zahlungserinnerung an den Kunden senden",
};
const FIXED = {
  "pl:inbox.unread": "{count, plural, one {# nieprzeczytana wiadomość} few {# nieprzeczytane wiadomości} many {# nieprzeczytanych wiadomości} other {# nieprzeczytanej wiadomości}}",
  "fr:home.welcome": "Bon retour, {name} !",
  "de:invoice.remind": "Erinnern",
};
const GOOD = {
  pl: { "home.welcome": "Witaj ponownie, {name}!", "invoice.remind": "Przypomnij" },
  fr: { "inbox.unread": "{count, plural, one {# message non lu} many {# messages non lus} other {# messages non lus}}", "invoice.remind": "Relancer" },
  de: { "inbox.unread": "{count, plural, one {# ungelesene Nachricht} other {# ungelesene Nachrichten}}", "home.welcome": "Willkommen zurück, {name}!" },
};

const payloadOf = (body) => {
  const u = body.messages.filter((m) => m.role === "user").map((m) => (typeof m.content === "string" ? m.content : m.content.map((c) => c.text ?? "").join(""))).join("\n");
  const a = u.indexOf("<payload>"), b = u.lastIndexOf("</payload>");
  return a < 0 ? null : JSON.parse(u.slice(a + 9, b));
};

async function upstream(body) {
  const r = await fetch(`${process.env.UPSTREAM.replace(/\/$/, "")}/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${process.env.UPSTREAM_KEY ?? ""}` },
    body: JSON.stringify(body),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(JSON.stringify(j));
  const t = j.choices[0].message.content;
  const a = t.indexOf("{"), b = t.lastIndexOf("}");
  const v = JSON.parse(t.slice(a, b + 1));
  return v.translations ?? v;
}

http
  .createServer(async (req, res) => {
    let raw = "";
    for await (const c of req) raw += c;
    const body = JSON.parse(raw);
    const p = payloadOf(body);
    const loc = p.target_locale;
    const retry = p.items.some((i) => i.rejected_attempt !== undefined);
    let out = {};
    if (process.env.UPSTREAM && (retry || p.items.some((i) => !BAD[`${loc}:${i.key}`]))) out = await upstream(body);
    for (const i of p.items) {
      const id = `${loc}:${i.key}`;
      if (i.rejected_attempt === undefined && BAD[id]) out[i.key] = BAD[id];
      else if (!process.env.UPSTREAM) out[i.key] = i.rejected_attempt === undefined ? GOOD[loc]?.[i.key] ?? i.source : FIXED[id];
      if (i.issues_to_fix) process.stderr.write(`[llm] ${id} retry, told: ${i.issues_to_fix.join(" | ")}\n`);
    }
    const content = JSON.stringify({ translations: out });
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ model: body.model, choices: [{ message: { role: "assistant", content } }], usage: { prompt_tokens: 0, completion_tokens: 0 } }));
  })
  .listen(Number(process.argv[2] ?? 18799));
