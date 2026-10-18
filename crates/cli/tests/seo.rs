//! `GLOT_SEO_E2E=1 cargo test -p glot-cli --test seo`: on a copy of examples/demo-saas,
//! init + i18nize + translate (mock) + `seo fix`, Next build and start, then `glot seo audit
//! --sitemap` must report 0 errors (and the unfixed hreflang must be caught before the fix).
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run(dir: &Path, cmd: &mut Command) -> (bool, String) {
    let out = cmd.current_dir(dir).env("NO_COLOR", "1").output().unwrap();
    let log =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), log)
}

fn glot(dir: &Path, args: &[&str]) -> (bool, String) {
    run(dir, Command::new(env!("CARGO_BIN_EXE_glot")).args(args))
}

struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", self.0.id())])
            .status();
        let _ = self.0.wait();
    }
}

/// Build + start on a free port; returns the server guard and its origin.
fn serve(dir: &Path) -> (Server, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let origin = format!("http://localhost:{port}");
    let (ok, log) = run(
        dir,
        Command::new("pnpm")
            .args(["exec", "next", "build"])
            .env("SITE_URL", &origin),
    );
    assert!(ok, "build failed:\n{log}");
    let child = std::os::unix::process::CommandExt::process_group(
        Command::new("pnpm")
            .args(["exec", "next", "start", "-p", &port.to_string()])
            .env("SITE_URL", &origin)
            .current_dir(dir),
        0,
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let s = Server(child);
    let t = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "server never answered"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    (s, origin)
}

#[test]
fn demo_saas_seo_audit_clean_after_fix() {
    if std::env::var("GLOT_SEO_E2E").is_err() {
        eprintln!("skipped: set GLOT_SEO_E2E=1 (needs pnpm + network, ~1 min)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-saas");
    let (ok, log) = run(
        dir,
        Command::new("sh").arg("-c").arg(format!(
            "tar -C {} --exclude node_modules --exclude .next --exclude .ready -cf - . | tar -xf -",
            app.display()
        )),
    );
    assert!(ok, "{log}");
    for args in [
        &[
            "init",
            "--yes",
            "--source",
            "en",
            "--targets",
            "fr,de,ja",
            "--provider",
            "mock",
        ][..],
        &["i18nize", "--yes"],
        &["translate", "--yes"],
    ] {
        let (ok, log) = glot(dir, args);
        assert!(ok, "glot {args:?}:\n{log}");
    }
    {
        let (_s, origin) = serve(dir);
        let (ok, log) = glot(
            dir,
            &[
                "seo",
                "audit",
                "--base-url",
                &origin,
                "--routes",
                "/,/pricing",
            ],
        );
        assert!(
            !ok && log.contains("hreflang_missing") && log.contains("canonical_missing"),
            "{log}"
        );
    }
    for args in [
        &["seo", "fix", "--yes"][..],
        &["seo", "keywords", "--apply"],
    ] {
        let (ok, log) = glot(dir, args);
        assert!(ok, "glot {args:?}:\n{log}");
    }
    assert!(
        glot(dir, &["seo", "fix", "--yes"])
            .1
            .contains("already in place")
    );
    let (_s, origin) = serve(dir);
    let (ok, log) = glot(dir, &["seo", "audit", "--base-url", &origin, "--sitemap"]);
    assert!(ok && log.contains("**0 errors**"), "{log}");
}
