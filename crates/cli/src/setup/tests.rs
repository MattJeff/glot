use super::*;

fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let path = dir.path().join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, c).unwrap();
    }
    dir
}

fn opts(routing: Routing) -> Opts {
    Opts {
        routing,
        switcher: true,
        locales: vec!["en".into(), "fr".into()],
        catalog: "locales/{locale}.json".into(),
    }
}

fn get<'a>(plan: &'a SetupPlan, path: &str) -> &'a str {
    &plan
        .changes
        .iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("no change for {path}: {:?}", paths(plan)))
        .new
}

fn paths(plan: &SetupPlan) -> Vec<&str> {
    plan.changes.iter().map(|c| c.path.as_str()).collect()
}

const LAYOUT: &str = r#"import "./globals.css";

export const metadata = { title: "Acme" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <main>{children}</main>
      </body>
    </html>
  );
}
"#;

#[test]
fn next_app_router_setup_is_complete_and_idempotent() {
    let dir = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"next":"16.4.2","react":"19"}}"#,
        ),
        ("pnpm-lock.yaml", ""),
        ("app/layout.tsx", LAYOUT),
        (
            "next.config.ts",
            "import type { NextConfig } from \"next\";\n\nconst nextConfig: NextConfig = {};\n\nexport default nextConfig;\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    assert_eq!(
        (p.framework, p.base, p.pm, p.next_major),
        (Framework::NextApp, "", "pnpm", 16)
    );
    let plan = super::plan(root, &p, &opts(Routing::Cookie));
    assert_eq!(
        paths(&plan),
        [
            "i18n/routing.ts",
            "i18n/request.ts",
            "i18n/proxy.ts",
            "proxy.ts",
            "next.config.ts",
            "app/layout.tsx",
            "components/LocaleSwitcher.tsx"
        ]
    );
    assert_eq!(plan.install_cmd().as_deref(), Some("pnpm add next-intl@^4"));
    assert!(get(&plan, "i18n/routing.ts").contains(r#"locales: ["en","fr"] as string[]"#));
    assert!(get(&plan, "i18n/routing.ts").contains(r#"localePrefix: "cookie""#));
    assert!(get(&plan, "i18n/request.ts").contains("import(`../locales/${locale}.json`)"));
    assert!(get(&plan, "proxy.ts").contains("export default function proxy(req: NextRequest)"));
    assert_eq!(
        get(&plan, "next.config.ts"),
        "import type { NextConfig } from \"next\";\nimport createNextIntlPlugin from \"next-intl/plugin\";\n\nconst nextConfig: NextConfig = {};\n\nexport default createNextIntlPlugin()(nextConfig);\n"
    );
    let layout = get(&plan, "app/layout.tsx");
    assert!(
        layout.contains("import { NextIntlClientProvider, useLocale } from \"next-intl\";"),
        "{layout}"
    );
    assert!(layout.contains("<html lang={useLocale()}>"), "{layout}");
    assert!(
        layout.contains("      <body>\n        <NextIntlClientProvider>\n        <main>{children}</main>\n        </NextIntlClientProvider>\n      </body>"),
        "{layout}"
    );
    assert!(get(&plan, "components/LocaleSwitcher.tsx").contains("from \"../i18n/routing\""));
    assert_eq!(plan.keys[0].0, "locale_switcher.label");

    // Second run: nothing left to do.
    write(root, &plan).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"dependencies":{"next":"16.4.2","next-intl":"^4"}}"#,
    )
    .unwrap();
    let p = detect(root).unwrap();
    let again = super::plan(root, &p, &opts(Routing::Cookie));
    assert!(
        again.changes.is_empty() && again.packages.is_empty(),
        "{:?}",
        paths(&again)
    );
    assert!(again.notes.is_empty(), "{:?}", again.notes);
}

#[test]
fn next_15_src_dir_cjs_config_async_layout_and_existing_middleware() {
    let dir = fixture(&[
        ("package.json", r#"{"dependencies":{"next":"^15.1.0"}}"#),
        ("package-lock.json", "{}"),
        (
            "src/app/layout.tsx",
            "export default async function L({ children }) {\n  return <html lang=\"en\"><body>{children}</body></html>;\n}\n",
        ),
        ("src/middleware.ts", "export function middleware() {}\n"),
        (
            "next.config.js",
            "/** @type {import('next').NextConfig} */\nmodule.exports = { reactStrictMode: true };\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    assert_eq!(
        (p.framework, p.base, p.pm, p.next_major),
        (Framework::NextApp, "src/", "npm", 15)
    );
    let plan = super::plan(root, &p, &opts(Routing::Prefix));
    assert_eq!(
        plan.install_cmd().as_deref(),
        Some("npm install next-intl@^4")
    );
    assert!(paths(&plan).contains(&"src/i18n/request.ts"));
    assert!(
        !paths(&plan).contains(&"src/proxy.ts") && !paths(&plan).contains(&"src/middleware.ts")
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("src/middleware.ts exists")),
        "{:?}",
        plan.notes
    );
    assert!(get(&plan, "src/i18n/routing.ts").contains(r#"localePrefix: "prefix""#));
    assert!(get(&plan, "src/i18n/request.ts").contains("import(`../../locales/${locale}.json`)"));
    assert_eq!(
        get(&plan, "next.config.js"),
        "const createNextIntlPlugin = require(\"next-intl/plugin\");\n/** @type {import('next').NextConfig} */\nmodule.exports = createNextIntlPlugin()({ reactStrictMode: true });\n"
    );
    let layout = get(&plan, "src/app/layout.tsx");
    assert!(layout.contains("<html lang={await getLocale()}><body><NextIntlClientProvider>{children}</NextIntlClientProvider></body>"), "{layout}");
    assert!(layout.starts_with("import { NextIntlClientProvider } from \"next-intl\";\nimport { getLocale } from \"next-intl/server\";\n"), "{layout}");
}

#[test]
fn next_pages_router() {
    let dir = fixture(&[
        ("package.json", r#"{"dependencies":{"next":"14.2.0"}}"#),
        ("yarn.lock", ""),
        (
            "pages/index.tsx",
            "export default function Home() { return <p>Hi</p>; }\n",
        ),
        (
            "pages/_app.tsx",
            "import type { AppProps } from \"next/app\";\n\nexport default function App({ Component, pageProps }: AppProps) {\n  return <><Header /><Component {...pageProps} /></>;\n}\n",
        ),
        (
            "next.config.mjs",
            "const nextConfig = {};\nexport default nextConfig;\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    assert_eq!((p.framework, p.pm), (Framework::NextPages, "yarn"));
    let plan = super::plan(root, &p, &opts(Routing::Cookie));
    assert!(get(&plan, "i18n/provider.tsx").contains("import fr from \"../locales/fr.json\";"));
    assert_eq!(get(&plan, "locales/fr.json"), "{}\n");
    assert_eq!(
        get(&plan, "next.config.mjs"),
        "const nextConfig = {};\nexport default { ...(nextConfig), i18n: { locales: [\"en\",\"fr\"], defaultLocale: \"en\" } };\n"
    );
    let app = get(&plan, "pages/_app.tsx");
    assert!(
        app.contains("import { I18nProvider } from \"../i18n/provider\";"),
        "{app}"
    );
    assert!(
        app.contains(
            "return <I18nProvider><><Header /><Component {...pageProps} /></></I18nProvider>;"
        ),
        "{app}"
    );
}

#[test]
fn react_i18next_vite_remix_astro() {
    let vite = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"react":"19","vite":"6"}}"#,
        ),
        ("bun.lock", ""),
        (
            "src/main.tsx",
            "import { createRoot } from \"react-dom/client\";\nimport App from \"./App\";\n\ncreateRoot(document.getElementById(\"root\")!).render(<App />);\n",
        ),
    ]);
    let p = detect(vite.path()).unwrap();
    assert_eq!((p.framework, p.pm), (Framework::Vite, "bun"));
    let o = Opts {
        catalog: "src/locales/{locale}.json".into(),
        ..opts(Routing::Cookie)
    };
    let plan = super::plan(vite.path(), &p, &o);
    assert_eq!(
        plan.install_cmd().as_deref(),
        Some("bun add i18next@^26 react-i18next@^17")
    );
    let i18n = get(&plan, "src/i18n.ts");
    assert!(
        i18n.contains("import l_fr from \"./locales/fr.json\";"),
        "{i18n}"
    );
    assert!(
        i18n.contains(
            "export const resources = { \"en\": { translation: l_en }, \"fr\": { translation: l_fr } };"
        ),
        "{i18n}"
    );
    assert!(
        get(&plan, "src/main.tsx").contains("import App from \"./App\";\nimport \"./i18n\";\n")
    );
    assert!(get(&plan, "src/components/LocaleSwitcher.tsx").contains("from \"../i18n\""));
    assert_eq!(get(&plan, "src/locales/en.json"), "{}\n");

    let remix = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"react":"19","react-router":"7"}}"#,
        ),
        (
            "app/root.tsx",
            "export default function Root() { return null; }\n",
        ),
    ]);
    let p = detect(remix.path()).unwrap();
    assert_eq!(p.framework, Framework::Remix);
    let plan = super::plan(remix.path(), &p, &o);
    assert!(get(&plan, "app/i18n.ts").contains("from \"../src/locales/en.json\""));
    assert!(get(&plan, "app/root.tsx").starts_with("import \"./i18n\";\n"));
    // React Router 7: remix-i18next v7 (`/middleware` entry) behind `future.v8_middleware`.
    assert!(get(&plan, "app/middleware/i18next.ts").contains("from \"remix-i18next/middleware\""));
    assert!(get(&plan, "react-router.config.ts").contains("future: { v8_middleware: true }"));
    assert!(plan.packages.contains(&"remix-i18next@^7".to_string()));

    let astro = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"astro":"5","react":"19"}}"#,
        ),
        (
            "src/components/Hello.tsx",
            "import { useTranslation } from \"react-i18next\";\nexport function Hello() { const { t } = useTranslation(); return <p>{t(\"hi\")}</p>; }\n",
        ),
        (
            "src/components/Plain.tsx",
            "export function Plain() { return null; }\n",
        ),
    ]);
    let p = detect(astro.path()).unwrap();
    assert_eq!(p.framework, Framework::Astro);
    let plan = super::plan(astro.path(), &p, &o);
    assert!(
        get(&plan, "src/components/Hello.tsx")
            .contains("from \"react-i18next\";\nimport \"../i18n\";")
    );
    assert!(!paths(&plan).contains(&"src/components/Plain.tsx"));
}

#[test]
fn relative_imports() {
    assert_eq!(
        rel_import("i18n", "locales/{locale}.json"),
        "../locales/{locale}.json"
    );
    assert_eq!(
        rel_import("src", "src/locales/en.json"),
        "./locales/en.json"
    );
    assert_eq!(
        rel_import("src/i18n", "messages/en.json"),
        "../../messages/en.json"
    );
}

#[test]
fn react_router_8_ssr_wiring() {
    let dir = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"react":"19","react-router":"^8.4.0","@react-router/node":"^8.4.0","isbot":"5"}}"#,
        ),
        ("pnpm-lock.yaml", ""),
        (
            "app/root.tsx",
            "import { Outlet, Scripts } from \"react-router\";\n\nexport function Layout({ children }: { children: React.ReactNode }) {\n  return (\n    <html lang=\"en\">\n      <body>{children}<Scripts /></body>\n    </html>\n  );\n}\n\nexport default function App() {\n  return <Outlet />;\n}\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    assert_eq!((p.framework, p.rr_major), (Framework::Remix, 8));
    let o = Opts {
        catalog: "app/locales/{locale}.json".into(),
        ..opts(Routing::Cookie)
    };
    let plan = super::plan(root, &p, &o);
    assert_eq!(
        plan.install_cmd().as_deref(),
        Some("pnpm add i18next@^26 react-i18next@^17 remix-i18next@^8")
    );
    // One change per file even though root.tsx is edited twice (i18n import + middleware).
    assert_eq!(
        paths(&plan)
            .iter()
            .filter(|p| **p == "app/root.tsx")
            .count(),
        1
    );
    let rootf = get(&plan, "app/root.tsx");
    assert!(rootf.contains("import \"./i18n\";"), "{rootf}");
    assert!(
        rootf.contains("<html lang={useTranslation().i18n.language}>"),
        "{rootf}"
    );
    assert!(
        rootf.contains("import { useTranslation } from \"react-i18next\";"),
        "{rootf}"
    );
    assert!(
        rootf.ends_with("export const middleware = [i18nextMiddleware];\n"),
        "{rootf}"
    );
    assert!(get(&plan, "app/middleware/i18next.ts").contains("from \"remix-i18next\";"));
    let entry = get(&plan, "app/entry.server.tsx");
    assert!(
        entry.contains("<I18nextProvider i18n={instance(loadContext)}>"),
        "{entry}"
    );
    let i18n = get(&plan, "app/i18n.ts");
    assert!(i18n.contains("document.documentElement.lang"), "{i18n}");
    assert!(i18n.contains("document.cookie"), "{i18n}");
    assert!(!paths(&plan).contains(&"react-router.config.ts"));

    write(root, &plan).unwrap();
    let again = super::plan(root, &p, &o);
    assert!(again.changes.is_empty(), "{:?}", paths(&again));
    assert!(again.notes.is_empty(), "{:?}", again.notes);
}

#[test]
fn astro_i18n_routing_and_middleware() {
    let dir = fixture(&[
        (
            "package.json",
            r#"{"dependencies":{"astro":"^7.4.0","@astrojs/react":"7","react":"19"}}"#,
        ),
        (
            "astro.config.mjs",
            "import { defineConfig } from \"astro/config\";\nimport react from \"@astrojs/react\";\n\nexport default defineConfig({\n  integrations: [react()],\n});\n",
        ),
        (
            "src/layouts/Layout.astro",
            "---\nconst { title } = Astro.props;\n---\n<html lang=\"en\">\n  <body><slot /></body>\n</html>\n",
        ),
        (
            "src/components/Hello.tsx",
            "import { useTranslation } from \"react-i18next\";\nexport function Hello() { const { t } = useTranslation(); return <p>{t(\"hi\")}</p>; }\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    let o = Opts {
        catalog: "src/locales/{locale}.json".into(),
        ..opts(Routing::Cookie)
    };
    let plan = super::plan(root, &p, &o);
    assert_eq!(
        get(&plan, "astro.config.mjs"),
        "import { defineConfig } from \"astro/config\";\nimport react from \"@astrojs/react\";\n\nexport default defineConfig({\n  integrations: [react()],\n  i18n: {\n    locales: [\"en\",\"fr\"],\n    defaultLocale: \"en\",\n    // Pages are written once: /fr/... renders them in French (glot i18nize).\n    fallback: { \"fr\": \"en\" },\n    routing: { prefixDefaultLocale: false, fallbackType: \"rewrite\" },\n  },\n});\n"
    );
    assert!(get(&plan, "src/layouts/Layout.astro").contains("<html lang={Astro.currentLocale}>"));
    assert!(get(&plan, "src/middleware.ts").contains("ctx.currentLocale"));
    assert!(get(&plan, "src/i18n.ts").contains("window.location.assign"));

    write(root, &plan).unwrap();
    let again = super::plan(root, &p, &o);
    assert!(again.changes.is_empty(), "{:?}", paths(&again));
    assert!(again.notes.is_empty(), "{:?}", again.notes);
}

#[test]
fn html_lang_rewrite() {
    assert_eq!(
        react::html_lang("<html lang='en' dir=\"ltr\">", "x").as_deref(),
        Some("<html lang={x} dir=\"ltr\">")
    );
    assert_eq!(
        react::html_lang("<html>", "x").as_deref(),
        Some("<html lang={x}>")
    );
    assert_eq!(react::html_lang("<div>", "x"), None);
}

#[test]
fn next_cache_components_moves_app_under_locale_segment() {
    let dir = fixture(&[
        ("package.json", r#"{"dependencies":{"next":"16.4.2"}}"#),
        (
            "app/layout.tsx",
            "import \"./globals.css\";\n\nexport default function RootLayout({ children }: LayoutProps<\"/\">) {\n  return (\n    <html lang=\"en\">\n      <body>{children}</body>\n    </html>\n  );\n}\n",
        ),
        ("app/globals.css", "body {}\n"),
        (
            "app/blog/page.tsx",
            "import { x } from \"../../lib/x\";\nimport { y } from \"../y\";\n\nexport default function Blog(p: PageProps<\"/blog\">) {\n  return null;\n}\n",
        ),
        (
            "app/api/hello/route.ts",
            "export const GET = () => new Response(\"hi\");\n",
        ),
        ("app/favicon.ico", "ico"),
        (
            "next.config.ts",
            "const nextConfig = {\n  cacheComponents: true,\n};\n\nexport default nextConfig;\n",
        ),
    ]);
    let root = dir.path();
    let p = detect(root).unwrap();
    let plan = super::plan(root, &p, &opts(Routing::Cookie));
    let moves: Vec<_> = plan.moves.iter().map(|(f, t)| format!("{f}>{t}")).collect();
    assert_eq!(
        moves,
        [
            "app/blog>app/[locale]/blog",
            "app/globals.css>app/[locale]/globals.css",
            "app/layout.tsx>app/[locale]/layout.tsx"
        ]
    );
    let blog = get(&plan, "app/[locale]/blog/page.tsx");
    assert!(
        blog.contains("from \"../../../lib/x\"") && blog.contains("from \"../y\""),
        "{blog}"
    );
    assert!(blog.contains("PageProps<\"/[locale]/blog\">"), "{blog}");
    let layout = get(&plan, "app/[locale]/layout.tsx");
    assert!(layout.contains("LayoutProps<\"/[locale]\">"), "{layout}");
    assert!(
        layout.contains("import { routing } from \"../../i18n/routing\";"),
        "{layout}"
    );
    assert!(
        layout.contains("export function generateStaticParams()"),
        "{layout}"
    );
    assert!(layout.contains("<NextIntlClientProvider>"), "{layout}");
    let request = get(&plan, "i18n/request.ts");
    assert!(
        request.contains("next/root-params") && !request.contains("headers()"),
        "{request}"
    );
    assert!(get(&plan, "i18n/proxy.ts").contains("NextResponse.rewrite(url)"));

    // Written to disk: moves first, then the edits on the moved files.
    super::write(root, &plan).unwrap();
    assert!(root.join("app/[locale]/blog/page.tsx").is_file() && !root.join("app/blog").exists());
    assert!(
        root.join("app/api/hello/route.ts").is_file() && root.join("app/favicon.ico").is_file()
    );
    // Idempotent: the segment exists, nothing moves again and the layout is already wired.
    let again = super::plan(root, &p, &opts(Routing::Cookie));
    assert!(again.moves.is_empty());
    assert!(
        !paths(&again).contains(&"app/[locale]/layout.tsx"),
        "{:?}",
        paths(&again)
    );
}
