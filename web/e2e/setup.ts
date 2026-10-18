import { expect, request, type FullConfig } from "@playwright/test";

// Signs up a throwaway user, creates a project with a few keys and a glossary term,
// saves the session for the tests and exposes the project slug as E2E_SLUG.
export default async function setup(config: FullConfig) {
  const slug = `a11y-${Date.now().toString(36)}`;
  const api = await request.newContext({ baseURL: config.projects[0].use.baseURL });
  const email = `${slug}@e2e.glot.test`;
  expect((await api.post("/api/auth/signup", { data: { email, password: "e2e-password-123", name: "A11y Bot" } })).ok()).toBe(true);
  const created = await api.post("/api/projects", {
    data: { name: "A11y project", slug, source_locale: "en", target_locales: ["fr", "de", "ar"] },
  });
  expect(created.ok()).toBe(true);
  const en: Record<string, string> = { "nav.home": "Home", "nav.billing": "Billing", "cart.items": "{count, plural, one {# item} other {# items}}", "cta.save": "Save {name}" };
  const fr: Record<string, string> = { "nav.home": "Accueil", "nav.billing": "Facturation", "cart.items": "{count, plural, one {# article} other {# articles}}", "cta.save": "Enregistrer" };
  // Long keys and values so the responsive audit sees realistic truncation and wrapping.
  for (let i = 0; i < 24; i++) {
    const k = `checkout.payment.methods.section_${i}.description_with_a_rather_long_key_name`;
    en[k] = `Pay securely with your saved card, Apple Pay or a bank transfer — section ${i} explains fees, refunds and limits in detail.`;
    if (i % 3) fr[k] = `Payez en toute sécurité avec votre carte enregistrée, Apple Pay ou un virement — la section ${i} détaille frais et limites.`;
  }
  expect((await api.post(`/api/projects/${slug}/sync/push`, { data: { catalogs: { en, fr }, glossary: [], context: {} } })).ok()).toBe(true);
  // Locale matrix: point the preview at this site's own login page so the iframes have something to render.
  await api.put(`/api/projects/${slug}/preview`, { data: { base_url: config.projects[0].use.baseURL, url_pattern: "/{locale}{route}", routes: ["/login"] } });
  // A job row (fails fast without an LLM key, which is fine for layout checks).
  await api.post(`/api/projects/${slug}/jobs`, { data: { kind: "translate", locales: ["de"] } });
  await api.post(`/api/projects/${slug}/glossary`, {
    data: { term: "glot", description: "Product name", do_not_translate: true, case_sensitive: true, translations: {}, forbidden: {} },
  });
  // OAuth consent needs a registered client; /oauth/* is not proxied, so it needs the API URL.
  if (process.env.E2E_API_URL) {
    const reg = await api.post(`${process.env.E2E_API_URL}/oauth/register`, {
      data: { client_name: "Responsive Bot", redirect_uris: ["http://127.0.0.1:9/cb"] },
    });
    if (reg.ok()) process.env.E2E_OAUTH_CLIENT = (await reg.json()).client_id;
  }
  await api.storageState({ path: "e2e/.auth.json" });
  await api.dispose();
  process.env.E2E_SLUG = slug;
}
