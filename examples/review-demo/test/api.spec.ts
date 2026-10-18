import { expect, request, test } from "@playwright/test";
import { API, SITE, anchor, json, member, state, widget } from "./env";

// Contract-level checks (docs/design/live-review-contract.md §1–3): what the widget, the SaaS page and the
// CLI/MCP all rely on. UI flows are in site.spec.ts / saas.spec.ts.

test("comment link: create, edit, react, delete its own annotation; server resolves the key from the text", async () => {
  const w = await widget(state().comment);
  const ctx = await json(await w.get("context"));
  expect(ctx.access).toBe("comment");

  const a = await json(await w.post("annotations", { data: { anchor: anchor(), body: "Too **long**, keep it short", author_name: "Lena" } }));
  expect(a).toMatchObject({ status: "open", key: "hero.title", locale: "de", route: "/", url: `${SITE}/de`, author: { name: "Lena" } });
  expect(a.body_html).toContain("<strong>long</strong>");

  const edited = await json(await w.patch(`annotations/${a.id}`, { data: { body: "Shorter please", author_name: "Lena" } }));
  expect(edited.body).toBe("Shorter please");
  const r = await json(await w.post(`annotations/${a.id}/reactions`, { data: { emoji: "+1", author_name: "Lena" } }));
  expect(r.reactions).toContainEqual(expect.objectContaining({ emoji: "+1", count: 1 }));
  // Status is reserved to members.
  expect((await w.patch(`annotations/${a.id}`, { data: { status: "resolved", author_name: "Lena" } })).status()).toBe(403);
  // Someone else on the same link (different name) cannot edit it.
  expect((await w.patch(`annotations/${a.id}`, { data: { body: "hijack", author_name: "Eve" } })).status()).toBe(403);
  expect((await w.delete(`annotations/${a.id}`, { data: { author_name: "Lena" } })).status()).toBe(204);
});

test("read link can read but not write; revoked or unknown tokens are refused; foreign origins too", async () => {
  const s = state();
  const r = await widget(s.read);
  expect((await json(await r.get("context"))).access).toBe("read");
  expect((await r.get("annotations")).ok()).toBe(true);
  expect((await r.post("annotations", { data: { anchor: anchor(), body: "x", author_name: "Ro" } })).status()).toBe(403);

  const bad = await widget("glr_not_a_real_token");
  expect((await bad.get("context")).status()).toBe(401);

  const evil = await request.newContext({ baseURL: `${API}/api/review/w/`, extraHTTPHeaders: { authorization: `Bearer ${s.comment}`, origin: "https://evil.example" } });
  expect((await evil.get("context")).status()).toBe(403);

  // Revocation is immediate.
  const m = await member();
  const { token, link } = await json(await m.post("links", { data: { access: "comment", label: "temp" } }));
  const t = await widget(token);
  expect((await t.get("context")).ok()).toBe(true);
  expect((await m.delete(`links/${link.id}`)).status()).toBe(204);
  expect((await t.get("context")).status()).toBe(401);
});

test("member: status, thread, apply a valid suggestion, refuse an invalid one", async () => {
  const m = await member();
  const a = await json(await m.post("annotations", {
    data: { anchor: anchor({ selector: "#hero-cta", text: "Jetzt kostenlos und unverbindlich testen", rect: { x: 560, y: 330, w: 176, h: 48 } }), body: "Overflows the button" },
  }));
  expect(a.key).toBe("hero.cta");
  await json(await m.post(`annotations/${a.id}/comments`, { data: { body: "Agreed, max ~20 chars" } }));
  expect((await json(await m.get(`annotations/${a.id}`))).comments).toHaveLength(1);
  expect((await json(await m.patch(`annotations/${a.id}`, { data: { status: "in_progress" } }))).status).toBe("in_progress");

  // Invalid: an ICU placeholder that does not exist in the source → validator error, nothing written.
  const bad = await m.post(`annotations/${a.id}/apply`, { data: { value: "Jetzt {gratis} testen" } });
  expect(bad.status()).toBe(422);
  expect((await bad.json()).error.code).toBe("invalid_translation");

  const done = await json(await m.post(`annotations/${a.id}/apply`, { data: { value: "Gratis testen" } }));
  expect(done.status).toBe("resolved");
  expect(done.translation).toMatchObject({ value: "Gratis testen", status: "reviewed" });
  const hist = await json(await m.get(`annotations/${a.id}/history`));
  expect(hist.map((e: any) => e.kind)).toContain("suggestion.applied");
  // Put the demo text back for the next run.
  await json(await m.put(`text/${encodeURIComponent("hero.cta")}`, { data: { values: { de: "Jetzt kostenlos und unverbindlich testen" } } }));
  await m.delete(`annotations/${a.id}`);
});

test("text that matches no key is still accepted (free annotation)", async () => {
  const w = await widget(state().comment);
  const a = await json(await w.post("annotations", { data: { anchor: anchor({ selector: "footer", text: "Not in any catalog" }), body: "Typo?", author_name: "Lena" } }));
  expect(a.key).toBeNull();
  await w.delete(`annotations/${a.id}`, { data: { author_name: "Lena" } });
});
