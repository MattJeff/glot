//! First contact, end to end: `create-next-app@latest` (Next 16+, `cacheComponents` on) ->
//! `glot init` -> `glot i18nize` -> `glot translate` (mock) -> `pnpm build` -> `/fr` is French in
//! the server HTML. Skipped when pnpm or the npm registry is unavailable.
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn run(dir: &Path, cmd: &mut Command) -> (bool, String) {
    let out = cmd
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("CI", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), log)
}

fn glot(dir: &Path, args: &[&str]) -> String {
    let (ok, log) = run(dir, Command::new(env!("CARGO_BIN_EXE_glot")).args(args));
    assert!(ok, "glot {args:?} failed:\n{log}");
    log
}

/// The server's process group, killed on drop (pnpm spawns node).
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", self.0.id())])
            .stderr(Stdio::null())
            .status();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(port: u16, path: &str, cookie: &str) -> Option<String> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut body = vec![];
    s.read_to_end(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

#[test]
fn create_next_app_to_french_page() {
    if Command::new("pnpm").arg("--version").output().is_err() {
        eprintln!("skipped: pnpm not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let (ok, log) = run(
        tmp.path(),
        Command::new("pnpm").args([
            "create",
            "next-app@latest",
            "app",
            "--yes",
            "--use-pnpm",
            "--skip-install",
            "--disable-git",
        ]),
    );
    if !ok {
        eprintln!("skipped: create-next-app failed (offline?)\n{log}");
        return;
    }
    let dir = tmp.path().join("app");
    let (ok, log) = run(
        &dir,
        Command::new("pnpm").args(["install", "--prefer-offline"]),
    );
    if !ok {
        eprintln!("skipped: `pnpm install` failed (offline?)\n{log}");
        return;
    }
    // A few first-contact patterns: plural ternary, module config mapped by the page.
    let page = dir.join("app/page.tsx");
    let src = std::fs::read_to_string(&page).unwrap();
    let src = src.replacen(
        "export default function Home() {",
        "const plans = [{ name: \"Starter plan\", description: \"For small teams getting started\" }];\n\n\
         export default function Home() {\n  const n = plans.length;",
        1,
    );
    let src = src.replacen(
        "<main",
        "<section><p>{n} {n === 1 ? \"item\" : \"items\"} in your cart</p>\
         {plans.map((p) => <p key={p.name}>{p.description}</p>)}</section>\n        <main",
        1,
    );
    std::fs::write(&page, src).unwrap();

    let init = glot(&dir, &["init", "--yes", "--provider", "mock"]);
    assert!(init.contains("next.js (no i18n library yet)"), "{init}");
    assert!(init.contains("glot i18nize"), "{init}");
    glot(&dir, &["i18nize", "--yes"]);
    glot(&dir, &["translate", "--yes"]);
    let en = std::fs::read_to_string(dir.join("messages/en.json")).unwrap();
    assert!(
        en.contains("{count, plural, one {# item} other {# items}} in your cart"),
        "{en}"
    );
    assert!(en.contains("For small teams getting started"), "{en}");
    let (ok, log) = run(&dir, Command::new("pnpm").arg("build"));
    assert!(ok, "build failed:\n{log}");
    assert!(log.contains("/fr"), "no static /fr route:\n{log}");

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _server = Server(
        std::os::unix::process::CommandExt::process_group(
            Command::new("pnpm")
                .args(["start", "-p", &port.to_string()])
                .current_dir(&dir),
            0,
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    let start = Instant::now();
    let html = loop {
        match get(port, "/fr", "") {
            Some(h) if h.starts_with("HTTP/1.1 200") => break h,
            _ if start.elapsed() > Duration::from_secs(60) => panic!("server never answered"),
            _ => std::thread::sleep(Duration::from_millis(300)),
        }
    };
    for want in [
        "<html lang=\"fr\"",
        "⟦fr⟧ Deploy Now",
        "⟦fr⟧ For small teams",
    ] {
        assert!(html.contains(want), "/fr lacks {want:?}:\n{html}");
    }
    let home = get(port, "/", "").unwrap();
    assert!(
        home.contains(">Deploy Now<") && home.contains("lang=\"en\""),
        "{home}"
    );
    // The cookie set by `/fr` keeps unprefixed URLs French.
    let again = get(port, "/", "NEXT_LOCALE=fr").unwrap();
    assert!(again.contains("⟦fr⟧ Deploy Now"), "{again}");
}
