//! Remote `$ref`s, served from `fixtures/remote` by a local server that routes like
//! `fixtures/remote/serve.mjs` (which produced `expected.ts` with openapi-typescript).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/remote")
}

fn respond(mut stream: TcpStream) {
    let mut request_line = String::new();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    reader.read_line(&mut request_line).unwrap();
    let mut header = String::new();
    while reader.read_line(&mut header).unwrap() > 2 {
        header.clear();
    }
    let path = request_line.split(' ').nth(1).unwrap_or("/").to_owned();
    let (status, headers, body) = if path == "/api/moved/owner.yaml" {
        ("302 Found", "location: /api/schemas/owner.yaml\r\n".to_owned(), Vec::new())
    } else {
        match std::fs::read(root().join(path.trim_start_matches('/'))) {
            Ok(body) => {
                let ty = if std::path::Path::new(&path).extension().is_some_and(|e| e == "yaml") {
                    "text/plain; charset=utf-8"
                } else {
                    "application/json"
                };
                ("200 OK", format!("content-type: {ty}\r\n"), body)
            }
            Err(_) => ("404 Not Found", String::new(), b"not found".to_vec()),
        }
    };
    let head = format!("HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
}

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            respond(stream);
        }
    });
    format!("http://{addr}")
}

fn run_cli(arg: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spectype")).arg(arg).output().unwrap()
}

#[test]
fn follows_relative_remote_refs() {
    let base = serve();
    let out = run_cli(&format!("{base}/api/openapi.yaml"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let expected = std::fs::read_to_string(root().join("expected.ts")).unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

#[test]
fn reports_the_url_that_failed() {
    let base = serve();
    let out = run_cli(&format!("{base}/api/missing.yaml"));
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains(&format!("Failed to load {base}/api/missing.yaml: 404 Not Found")), "{stderr}");
}

#[test]
fn file_urls_are_paths() {
    let path = root().join("api/moved/date.yaml").canonicalize().unwrap();
    let doc = std::env::temp_dir().join(format!("spectype-file-url-{}.yaml", std::process::id()));
    std::fs::write(
        &doc,
        format!(
            "openapi: 3.1.0\ninfo: {{title: t, version: '1'}}\ncomponents:\n  schemas:\n    Pet:\n      $ref: '{}'\n",
            path.display()
        ),
    )
    .unwrap();
    let out = run_cli(&format!("file://{}", doc.display()));
    let _ = std::fs::remove_file(&doc);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8(out.stdout).unwrap().contains("Pet: components[\"schemas\"][\"date\"];"));
}
