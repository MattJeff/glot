//! AI-builder i18n (anonymized from the Lovable / Bolt / v0 sample, docs/research/ai-generated-i18n.md):
//! a `LanguageContext.tsx` holding its own dictionary and `useLanguage().t`, language ternaries,
//! and a TanStack Start route. glot adopts the app's runtime instead of adding a second one.
use std::path::Path;
use std::process::Command;

fn glot(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glot"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), log)
}

fn write(r: &Path, p: &str, c: &str) {
    std::fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
    std::fs::write(r.join(p), c).unwrap();
}

const CONTEXT: &str = "import React, { createContext, useContext, useState } from 'react';
type Language = 'en' | 'fr';
const translations: Record<Language, Record<string, string>> = {
  en: { 'nav.home': 'Home', 'hero.title': 'Build faster' },
  fr: { 'nav.home': 'Accueil', 'hero.title': 'Construisez plus vite' },
};
interface Ctx { language: Language; setLanguage: (l: Language) => void; t: (k: string) => string }
const LanguageContext = createContext<Ctx | undefined>(undefined);
export const LanguageProvider = ({ children }: { children: React.ReactNode }) => {
  const [language, setLanguage] = useState<Language>(() => (localStorage.getItem('app-lang') as Language) || 'en');
  const t = (key: string) => translations[language][key] || key;
  return <LanguageContext.Provider value={{ language, setLanguage, t }}>{children}</LanguageContext.Provider>;
};
export const useLanguage = () => { const c = useContext(LanguageContext); if (!c) throw new Error('no provider'); return c; };
";

#[test]
fn language_context_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        "package.json",
        r#"{"dependencies":{"react":"18"},"devDependencies":{"vite":"5","lovable-tagger":"1"}}"#,
    );
    write(r, "src/contexts/LanguageContext.tsx", CONTEXT);
    write(
        r,
        "src/components/Hero.tsx",
        "import { useLanguage } from '@/contexts/LanguageContext';\nexport const Hero = () => {\n  const { t, language } = useLanguage();\n  return <section><h1>{t('hero.title')}</h1><p>No credit card required</p><h2>{language === 'fr' ? 'Nos tarifs' : 'Our pricing'}</h2></section>;\n};\n",
    );

    let (ok, log) = glot(r, &["init", "--yes", "--provider", "mock"]);
    assert!(
        ok && log.contains("stack: custom (LanguageContext)"),
        "{log}"
    );
    assert!(log.contains("agent-prompt --for lovable"), "{log}");
    // Rules for every agent, AGENTS.md created, adapted to the app's own runtime.
    for f in [
        "CLAUDE.md",
        "AGENTS.md",
        ".claude/skills/glot/SKILL.md",
        ".cursor/rules/glot.mdc",
    ] {
        let s = std::fs::read_to_string(r.join(f)).unwrap();
        assert!(
            s.contains("`const { t } = useLanguage()`")
                && s.contains("do NOT create a LanguageContext"),
            "{f}: {s}"
        );
        assert!(!s.contains("useTranslation"), "{f}: {s}");
    }
    assert!(!r.join("replit.md").exists());
    let (ok, log) = glot(r, &["status"]);
    assert!(ok && log.contains("2/2"), "{log}");
    let (_, log) = glot(r, &["check"]);
    assert!(
        log.contains("hand_rolled_i18n") && log.contains("src/components/Hero.tsx"),
        "{log}"
    );

    // i18nize: the app's own t(), the ternary keeps its French text as an imported translation.
    let (ok, log) = glot(r, &["i18nize", "--yes"]);
    assert!(ok, "{log}");
    let hero = std::fs::read_to_string(r.join("src/components/Hero.tsx")).unwrap();
    assert!(
        !hero.contains("useTranslation") && hero.matches("useLanguage()").count() == 1,
        "{hero}"
    );
    assert!(
        !hero.contains("language === 'fr'") && !hero.contains("No credit card"),
        "{hero}"
    );
    let ctx = std::fs::read_to_string(r.join("src/contexts/LanguageContext.tsx")).unwrap();
    assert!(
        ctx.contains("'Nos tarifs'") && ctx.contains("'Our pricing'"),
        "{ctx}"
    );
    let lock = std::fs::read_to_string(r.join("glot.lock")).unwrap();
    assert!(lock.contains("\"imported\""), "{lock}");
    assert!(
        !r.join("src/i18n.ts").exists()
            && !std::fs::read_to_string(r.join("package.json"))
                .unwrap()
                .contains("i18next")
    );

    // A new locale: dictionary block (flat keys like its siblings) and the `Language` union.
    let toml = std::fs::read_to_string(r.join("glot.toml")).unwrap();
    std::fs::write(
        r.join("glot.toml"),
        toml.replace(
            "target_locales = [\"fr\"]",
            "target_locales = [\"fr\",\"de\"]",
        ),
    )
    .unwrap();
    let (ok, log) = glot(r, &["translate", "--yes"]);
    assert!(ok, "{log}");
    let ctx2 = std::fs::read_to_string(r.join("src/contexts/LanguageContext.tsx")).unwrap();
    assert!(
        ctx2.contains("type Language = 'en' | 'fr' | 'de';"),
        "{ctx2}"
    );
    assert!(ctx2.contains("    'nav.home': '⟦de⟧ Home',"), "{ctx2}");
    // The French imported from the ternary was not retranslated.
    assert!(ctx2.contains("'Nos tarifs'"), "{ctx2}");
    // Everything outside the dictionary is byte for byte the same.
    let tail = |s: &str| s[s.find("interface Ctx").unwrap()..].to_string();
    assert_eq!(tail(&ctx2), tail(CONTEXT));
    let (ok, log) = glot(r, &["check"]);
    assert!(ok && !log.contains("hand_rolled_i18n"), "{log}");
}

#[test]
fn tanstack_start_route_and_entry() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        "package.json",
        r#"{"type":"module","dependencies":{"react":"19","@tanstack/react-start":"1","@tanstack/react-router":"1"},"devDependencies":{"vite":"7"}}"#,
    );
    write(
        r,
        "src/routes/__root.tsx",
        "import { createRootRoute, Outlet } from '@tanstack/react-router';\nexport const Route = createRootRoute({ component: () => <Outlet /> });\n",
    );
    write(
        r,
        "src/routes/index.tsx",
        "import { createFileRoute } from '@tanstack/react-router';\nexport const Route = createFileRoute('/')({ component: () => <h1>Welcome to the dashboard</h1> });\n",
    );
    write(r, "src/router.tsx", "export function getRouter() {}\n");
    let (ok, log) = glot(
        r,
        &["init", "--yes", "--provider", "mock", "--targets", "fr"],
    );
    assert!(ok, "{log}");
    let (ok, log) = glot(r, &["i18nize", "--yes", "--no-install"]);
    assert!(ok && log.contains("TanStack Start"), "{log}");
    let index = std::fs::read_to_string(r.join("src/routes/index.tsx")).unwrap();
    assert!(index.contains("const { t } = useTranslation();"), "{index}");
    let root = std::fs::read_to_string(r.join("src/routes/__root.tsx")).unwrap();
    assert!(root.contains("import \"../i18n\";"), "{root}");
    assert!(!log.contains("no src/main.tsx"), "{log}");
}
