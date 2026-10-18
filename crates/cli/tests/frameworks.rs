//! End to end on the real apps of `examples/frameworks/`: `glot init` + `glot i18nize` +
//! `glot translate` (mock provider), production build, server, then the French page must be
//! French in the server HTML (SSR) and, with Playwright, after hydration without console errors.
//! Skipped when pnpm or the npm registry is unavailable. Default app: react-router (SSR via
//! remix-i18next); `GLOT_FW_APPS=react-router,astro,vite-react,next-pages` runs them all.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        let skip = [
            "node_modules",
            "build",
            "dist",
            ".next",
            ".astro",
            ".react-router",
        ];
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        if e.path().is_dir() {
            copy(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

fn run(dir: &Path, cmd: &mut Command) -> (bool, String) {
    let out = cmd.current_dir(dir).env("NO_COLOR", "1").output().unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), log)
}

fn glot(dir: &Path, args: &[&str]) {
    let (ok, log) = run(dir, Command::new(env!("CARGO_BIN_EXE_glot")).args(args));
    assert!(ok, "glot {args:?} failed:\n{log}");
}

/// The server's whole process group, killed on drop (pnpm spawns node).
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

fn get(port: u16, path: &str) -> Option<String> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut body = vec![];
    s.read_to_end(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn scenario(app: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(app);
    copy(&repo().join("examples/frameworks").join(app), &dir);
    let (ok, log) = run(
        &dir,
        Command::new("pnpm").args(["install", "--prefer-offline"]),
    );
    if !ok {
        eprintln!("skipped {app}: `pnpm install` failed (offline?)\n{log}");
        return;
    }
    glot(&dir, &["init", "--yes", "--provider", "mock"]);
    glot(&dir, &["i18nize", "--yes"]);
    glot(&dir, &["translate", "--yes"]);
    let (ok, log) = run(&dir, Command::new("pnpm").arg("build"));
    assert!(ok, "{app}: build failed:\n{log}");

    let port = free_port();
    let p = port.to_string();
    let (args, path, ssr): (Vec<&str>, &str, bool) = match app {
        "react-router" => (vec!["start"], "/?lng=fr", true),
        "astro" => (
            vec!["preview", "--host", "127.0.0.1", "--port", &p],
            "/fr/",
            true,
        ),
        "next-pages" => (vec!["start", "-p", &p], "/fr", true),
        _ => (
            vec![
                "preview",
                "--host",
                "127.0.0.1",
                "--port",
                &p,
                "--strictPort",
            ],
            "/?lng=fr",
            false,
        ),
    };
    let server = Server(
        std::os::unix::process::CommandExt::process_group(
            Command::new("pnpm")
                .args(&args)
                .env("PORT", &p)
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
        match get(port, path) {
            Some(h) if h.starts_with("HTTP/1.1 200") => break h,
            _ if start.elapsed() > Duration::from_secs(60) => {
                panic!("{app}: server never answered")
            }
            _ => std::thread::sleep(Duration::from_millis(300)),
        }
    };
    if ssr {
        for want in [
            "<html lang=\"fr\"",
            "⟦fr⟧ Ship your projects faster",
            "⟦fr⟧ You have 1 open task",
        ] {
            assert!(
                html.contains(want),
                "{app}: server HTML lacks {want:?}:\n{html}"
            );
        }
    }

    let pw = repo().join("packages/visual-qa/node_modules/playwright");
    if !pw.exists() {
        eprintln!("browser check skipped: run `pnpm install` in packages/visual-qa");
        return;
    }
    let check = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fw_check.mjs");
    let url = format!("http://localhost:{port}{path}");
    let (ok, log) = run(&dir, Command::new("node").arg(&check).arg(&pw).arg(&url));
    drop(server);
    assert!(ok, "{app}: browser check failed:\n{log}");
    eprintln!("{app}: {}", log.trim());
}

#[test]
fn framework_apps_render_french_on_server_and_client() {
    if Command::new("pnpm").arg("--version").output().is_err() {
        eprintln!("skipped: pnpm not installed");
        return;
    }
    let apps = std::env::var("GLOT_FW_APPS").unwrap_or_else(|_| "react-router".into());
    for app in apps.split(',') {
        scenario(app.trim());
    }
}
