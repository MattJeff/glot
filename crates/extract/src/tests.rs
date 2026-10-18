use super::*;
use glot_core::{catalog::Catalog, config::ExtractConfig, lock::Lockfile};

fn scan(path: &str, src: &str) -> ExtractReport {
    extract_source(path, src, &ExtractConfig::default())
}

fn line_of(r: &ExtractReport, key: &str) -> Vec<u32> {
    r.usages
        .get(key)
        .unwrap_or_else(|| panic!("no usage for {key}: {:?}", r.usages.keys()))
        .iter()
        .map(|u| u.at.line)
        .collect()
}

fn catalog(entries: &[(&str, &str)]) -> Catalog {
    let mut c = Catalog::new("en");
    c.entries = entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    c
}

const NEXT_INTL_PAGE: &str = r#"import { useTranslations } from "next-intl";
import { getTranslations } from "next-intl/server";

export async function generateMetadata() {
  const t = await getTranslations({ locale: "en", namespace: "settings.billing" });
  return { title: t("meta_title") };
}

export default function BillingPage() {
  const t = useTranslations("settings.billing");
  const errors = useTranslations("errors");
  return (
    <section>
      <h1>{t("title")}</h1>
      <p>{t.rich("intro", { b: (chunks) => <b>{chunks}</b> })}</p>
      <input placeholder={t("card_placeholder")} />
      {t.has("promo") && <span>{t.raw("promo")}</span>}
      <button onClick={() => toast(errors("saved"))}>{t("save")}</button>
      <p>{errors(`codes.${code}`)}</p>
    </section>
  );
}
"#;

#[test]
fn next_intl_namespaces_roles_and_lines() {
    let r = scan("app/(app)/settings/billing/page.tsx", NEXT_INTL_PAGE);
    assert_eq!(r.files_scanned, 1);
    assert_eq!(line_of(&r, "settings.billing.meta_title"), [6]);
    assert_eq!(line_of(&r, "settings.billing.title"), [14]);
    assert_eq!(line_of(&r, "settings.billing.intro"), [15]);
    assert_eq!(line_of(&r, "settings.billing.promo"), [17, 17]);
    let title = &r.usages["settings.billing.title"][0];
    assert_eq!(title.ui_role.as_deref(), Some("heading"));
    assert_eq!(title.component.as_deref(), Some("BillingPage"));
    assert_eq!(
        r.usages["settings.billing.card_placeholder"][0]
            .ui_role
            .as_deref(),
        Some("placeholder")
    );
    assert_eq!(
        r.usages["settings.billing.save"][0].ui_role.as_deref(),
        Some("button")
    );
    assert_eq!(
        r.usages["errors.saved"][0].ui_role.as_deref(),
        Some("toast")
    );
    assert_eq!(r.dynamic.len(), 1);
    assert_eq!(r.dynamic[0].line, 19);
    assert!(r.dynamic_prefixes.contains("errors.codes."));
    assert!(r.hardcoded.is_empty(), "{:?}", r.hardcoded);
}

const I18NEXT_COMPONENT: &str = r#"import { useTranslation, Trans } from "react-i18next";
import i18n from "./i18n";

export const LoginForm = () => {
  const { t } = useTranslation("auth");
  const { t: tf } = useTranslation("forms", { keyPrefix: "login" });
  const label = cond ? t("common:yes") : t("common:no");
  alert(i18n.t("session.expired"));
  return (
    <form>
      <label>{tf("email")}</label>
      <Trans i18nKey="auth:welcome">Welcome <b>back</b></Trans>
      <button>{t(dynamicKey)}</button>
    </form>
  );
};
"#;

#[test]
fn react_i18next_ns_and_key_prefix() {
    let r = scan("src/components/LoginForm.tsx", I18NEXT_COMPONENT);
    let keys: Vec<&str> = r.usages.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["login.email", "no", "session.expired", "welcome", "yes"]
    );
    assert_eq!(r.usages["login.email"][0].ui_role.as_deref(), Some("label"));
    assert_eq!(
        r.usages["login.email"][0].component.as_deref(),
        Some("LoginForm")
    );
    assert_eq!(line_of(&r, "welcome"), [12]);
    // `t(dynamicKey)` without keyPrefix: dynamic, but no prefix to protect.
    assert_eq!(r.dynamic.len(), 1);
    assert!(r.dynamic_prefixes.is_empty());
    // Trans children are default values, not hardcoded copy.
    assert!(r.hardcoded.is_empty(), "{:?}", r.hardcoded);
}

const FORMATJS: &str = r#"import { defineMessages, FormattedMessage, useIntl } from "react-intl";

const messages = defineMessages({
  greeting: { id: "home.greeting", defaultMessage: "Hello {name}" },
  cta: { id: "home.cta", defaultMessage: "Start" },
});

export function Home({ name }) {
  const intl = useIntl();
  return (
    <main>
      <h2><FormattedMessage id="home.title" defaultMessage="Home" /></h2>
      <img alt={intl.formatMessage({ id: "home.hero_alt" })} src="/hero.png" />
      {intl.formatMessage(messages.greeting, { name })}
    </main>
  );
}
"#;

#[test]
fn formatjs_ids() {
    let r = scan("src/Home.jsx", FORMATJS);
    let keys: Vec<&str> = r.usages.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["home.cta", "home.greeting", "home.hero_alt", "home.title"]
    );
    assert_eq!(line_of(&r, "home.greeting"), [4]);
    assert_eq!(
        r.usages["home.title"][0].ui_role.as_deref(),
        Some("heading")
    );
    assert_eq!(
        r.usages["home.hero_alt"][0].ui_role.as_deref(),
        Some("aria")
    );
    // `formatMessage(messages.greeting)` is a reference, not a literal key.
    assert_eq!(r.dynamic.len(), 1);
}

#[test]
fn vue_script_t() {
    let src = "<template>\n  <p>{{ $t('ignored.in_template') }}</p>\n</template>\n<script setup lang=\"ts\">\nconst { t } = useI18n();\nconst msg: string = t('nav.home');\nconsole.log(this.$t('nav.about'));\n</script>\n";
    let r = scan("src/App.vue", src);
    assert_eq!(line_of(&r, "nav.home"), [6]);
    assert_eq!(line_of(&r, "nav.about"), [7]);
    assert_eq!(r.usages.len(), 2);
}

const HARDCODED: &str = r#"import Link from "next/link";

export default function Navbar() {
  return (
    <nav className="flex items-center gap-2">
      <Link href="/pricing">Pricing</Link>
      <h2 id="main-title">
        Manage your   team
      </h2>
      <input placeholder="Search projects" type="email" name="userEmail" />
      <button aria-label="Close menu">×</button>
      <Button>Save changes</Button>
      <span>{"Créer un espace"}</span>
      <p>42 — {count}</p>
      <code>npm install glot</code>
      <a href="mailto:hi@glot.dev">hi@glot.dev</a>
      <p>getUserById</p>
      <p>/settings/billing</p>
    </nav>
  );
}
"#;

#[test]
fn hardcoded_strings() {
    let r = scan("components/Navbar.tsx", HARDCODED);
    let got: Vec<(&str, &str, &str, u32)> = r
        .hardcoded
        .iter()
        .map(|h| {
            (
                h.text.as_str(),
                h.ui_role.as_deref().unwrap(),
                h.suggested_key.as_str(),
                h.at.line,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("Pricing", "link", "navbar.pricing", 6),
            ("Manage your team", "heading", "navbar.manage_team_title", 8),
            (
                "Search projects",
                "placeholder",
                "navbar.search_projects_placeholder",
                10
            ),
            ("Close menu", "aria", "navbar.close_menu_aria", 11),
            ("Save changes", "button", "navbar.save_changes_button", 12),
            ("Créer un espace", "text", "navbar.creer_un_espace", 13),
        ]
    );
    assert!(
        r.hardcoded
            .iter()
            .all(|h| h.component.as_deref() == Some("Navbar"))
    );
}

#[test]
fn hardcoded_skipped_in_tests() {
    assert!(
        scan("components/Navbar.test.tsx", HARDCODED)
            .hardcoded
            .is_empty()
    );
}

#[test]
fn suggested_key_prefixes() {
    use crate::scan::suggest_key;
    assert_eq!(
        suggest_key("app/(app)/settings/billing/page.tsx", "Save", "button"),
        "settings.billing.save_button"
    );
    assert_eq!(
        suggest_key(
            "src/app/[locale]/page.tsx",
            "Welcome to the app!",
            "heading"
        ),
        "common.welcome_app_title"
    );
    assert_eq!(
        suggest_key(
            "src/features/UserProfile.tsx",
            "Delete my account now please",
            "text"
        ),
        "features.user_profile.delete_account_now_please"
    );
    assert!(suggest_key("components/Hero.tsx", "ようこそ", "text").starts_with("hero.text_"));
}

#[test]
fn unused_and_missing() {
    let r = scan("app/(app)/settings/billing/page.tsx", NEXT_INTL_PAGE);
    let source = catalog(&[
        ("settings.billing.meta_title", "Billing"),
        ("settings.billing.title", "Billing"),
        ("settings.billing.intro", "Your <b>plan</b>"),
        ("settings.billing.promo", "Promo"),
        ("settings.billing.save", "Save"),
        ("settings.billing.legacy", "Old"),
        ("errors.codes.card_declined", "Declined"),
        ("errors.unknown", "Oops"),
    ]);
    assert_eq!(
        unused_keys(&r, &source),
        ["errors.unknown", "settings.billing.legacy"]
    );
    assert_eq!(
        missing_keys(&r, &source),
        ["errors.saved", "settings.billing.card_placeholder"]
    );

    // i18next plurals and subtree usage.
    let r = scan(
        "src/a.ts",
        "t('cart.items', { count }); t.raw; const t2 = t('faq');",
    );
    let source = catalog(&[
        ("cart.items_one", "1 item"),
        ("cart.items_other", "{{count}} items"),
        ("faq.q1", "?"),
    ]);
    assert!(unused_keys(&r, &source).is_empty());
    assert!(missing_keys(&r, &source).is_empty());
}

#[test]
fn contexts_from_usages_notes_and_siblings() {
    let r = scan("app/(app)/settings/billing/page.tsx", NEXT_INTL_PAGE);
    let source = catalog(&[
        ("settings.billing.title", "Billing"),
        ("settings.billing.save", "Save"),
        ("settings.billing.plan.name", "Plan"),
        ("settings.profile", "Profile"),
    ]);
    let mut lock = Lockfile::default();
    lock.notes
        .insert("settings.billing.save".into(), "Primary CTA".into());
    let ctx = contexts(&r, &source, &lock);
    let save = &ctx["settings.billing.save"];
    assert_eq!(save.description.as_deref(), Some("Primary CTA"));
    assert_eq!(save.ui_role.as_deref(), Some("button"));
    assert_eq!(save.component.as_deref(), Some("BillingPage"));
    assert_eq!(save.refs[0].line, 18);
    assert_eq!(
        save.neighbors,
        [("settings.billing.title".to_string(), "Billing".to_string())]
    );
    assert!(ctx.contains_key("settings.profile"));
    assert!(ctx["settings.profile"].refs.is_empty());
}

#[test]
fn extract_walks_with_globs() {
    let root = std::env::temp_dir().join(format!("glot-extract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |rel: &str, src: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    };
    write("app/(app)/settings/billing/page.tsx", NEXT_INTL_PAGE);
    write("components/Navbar.tsx", HARDCODED);
    write("components/Navbar.test.tsx", "t('from.test')");
    write("node_modules/lib/app/x.tsx", "t('from.node_modules')");
    write("app/.next/x.tsx", "t('from.next')");
    write("app/styles.css", "body {}");

    let cfg = ExtractConfig {
        include: vec!["app/**/*.{ts,tsx}".into(), "components/**/*.tsx".into()],
        ..Default::default()
    };
    let r = extract(&root, &cfg).unwrap();
    std::fs::remove_dir_all(&root).unwrap();

    assert_eq!(r.files_scanned, 2);
    assert!(r.usages.contains_key("settings.billing.title"));
    assert!(!r.usages.keys().any(|k| k.starts_with("from.")));
    assert_eq!(r.hardcoded.len(), 6);
    assert_eq!(
        r.usages["settings.billing.title"][0].at.path,
        "app/(app)/settings/billing/page.tsx"
    );
}

#[test]
fn brace_expansion() {
    assert_eq!(
        crate::expand_braces("src/**/*.{ts,tsx}"),
        ["src/**/*.ts", "src/**/*.tsx"]
    );
    assert_eq!(crate::expand_braces("{a,b}/*.{x,y}").len(), 4);
}

#[test]
fn translate_no_opts_out() {
    let r = scan(
        "app/page.tsx",
        r#"export default function P() {
  return <div>
    <span translate="no">npm install acme</span>
    <input translate="no" placeholder="x-team: growth" />
    <p>Real copy here</p>
  </div>;
}"#,
    );
    let texts: Vec<_> = r.hardcoded.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(texts, ["Real copy here"]);
}
