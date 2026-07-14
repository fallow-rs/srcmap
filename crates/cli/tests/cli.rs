use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

fn srcmap() -> Command {
    Command::new(env!("CARGO_BIN_EXE_srcmap"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn write_sources_map(dir: &Path, sources: &[&str]) -> PathBuf {
    let path = dir.join("security.js.map");
    let contents: Vec<String> =
        sources.iter().map(|source| format!("malicious content for {source}")).collect();
    let map = serde_json::json!({
        "version": 3,
        "file": "security.js",
        "sources": sources,
        "names": [],
        "mappings": "",
        "sourcesContent": contents,
    });
    fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
    path
}

fn extract_sources(map: &Path, output_dir: &Path) -> serde_json::Value {
    let out = srcmap()
        .args([
            "sources",
            map.to_str().unwrap(),
            "--extract",
            "-o",
            output_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

// ── info ─────────────────────────────────────────────────────────

#[test]
fn info_human() {
    let out = srcmap().args(["info", fixture("simple.js.map").to_str().unwrap()]).output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success());
    assert!(stdout.contains("File:         simple.js"));
    assert!(stdout.contains("Sources:      2"));
    assert!(stdout.contains("Names:        3"));
    assert!(stdout.contains("Mappings:     14"));
}

#[test]
fn info_json() {
    let out = srcmap()
        .args(["info", fixture("simple.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["file"], "simple.js");
    assert_eq!(v["sources"], 2);
    assert_eq!(v["names"], 3);
    assert_eq!(v["mappings"], 14);
    assert_eq!(v["lines"], 2);
}

// ── validate ─────────────────────────────────────────────────────

#[test]
fn validate_valid() {
    let out =
        srcmap().args(["validate", fixture("simple.js.map").to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Valid source map v3"));
}

#[test]
fn validate_valid_json() {
    let out = srcmap()
        .args(["validate", fixture("simple.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

#[test]
fn validate_invalid() {
    let out =
        srcmap().args(["validate", fixture("invalid.js.map").to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("Invalid source map"));
}

#[test]
fn validate_invalid_json() {
    let out = srcmap()
        .args(["validate", fixture("invalid.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    // validate exits with failure for invalid maps
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
    assert!(v["error"].as_str().unwrap().contains("VLQ"));
}

// ── lookup ───────────────────────────────────────────────────────

#[test]
fn lookup_found() {
    let out = srcmap()
        .args(["lookup", fixture("simple.js.map").to_str().unwrap(), "0", "0", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["source"], "src/app.ts");
    assert_eq!(v["line"], 0);
    assert_eq!(v["column"], 0);
}

#[test]
fn lookup_not_found() {
    let out = srcmap()
        .args(["lookup", fixture("simple.js.map").to_str().unwrap(), "999", "0"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("no mapping found"));
}

// ── resolve ──────────────────────────────────────────────────────

#[test]
fn resolve_found() {
    let out = srcmap()
        .args([
            "resolve",
            fixture("simple.js.map").to_str().unwrap(),
            "--source",
            "src/app.ts",
            "0",
            "0",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["line"].is_number());
    assert!(v["column"].is_number());
}

#[test]
fn resolve_not_found() {
    let out = srcmap()
        .args([
            "resolve",
            fixture("simple.js.map").to_str().unwrap(),
            "--source",
            "nonexistent.ts",
            "0",
            "0",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ── decode / encode ──────────────────────────────────────────────

#[test]
fn decode_inline() {
    let out = srcmap().args(["decode", "AAAA;AACA"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let lines = v.as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0][0], serde_json::json!([0, 0, 0, 0]));
    assert_eq!(lines[1][0], serde_json::json!([0, 0, 1, 0]));
}

#[test]
fn decode_compact() {
    let out = srcmap().args(["decode", "AAAA", "--compact"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Compact output should be a single line
    assert_eq!(stdout.trim().lines().count(), 1);
}

#[test]
fn encode_from_stdin() {
    let out = srcmap()
        .arg("encode")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"[[[0,0,0,0]],[[0,0,1,0]]]").unwrap();
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "AAAA;AACA");
}

#[test]
fn encode_json_output() {
    let out = srcmap()
        .args(["encode", "--json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"[[[0,0,0,0]]]").unwrap();
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["vlq"], "AAAA");
}

// ── mappings ─────────────────────────────────────────────────────

#[test]
fn mappings_json() {
    let out = srcmap()
        .args(["mappings", fixture("simple.js.map").to_str().unwrap(), "--limit", "3", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["total"], 14);
    assert_eq!(v["limit"], 3);
    assert_eq!(v["hasMore"], true);
    assert_eq!(v["mappings"].as_array().unwrap().len(), 3);
}

#[test]
fn mappings_with_source_filter() {
    let out = srcmap()
        .args([
            "mappings",
            fixture("simple.js.map").to_str().unwrap(),
            "--source",
            "src/app.ts",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for m in v["mappings"].as_array().unwrap() {
        assert_eq!(m["source"], "src/app.ts");
    }
}

#[test]
fn mappings_source_not_found() {
    let out = srcmap()
        .args([
            "mappings",
            fixture("simple.js.map").to_str().unwrap(),
            "--source",
            "nonexistent.ts",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ── concat ───────────────────────────────────────────────────────

#[test]
fn concat_dry_run() {
    let out = srcmap()
        .args([
            "concat",
            fixture("simple.js.map").to_str().unwrap(),
            fixture("second.js.map").to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["inputFiles"].as_array().unwrap().len(), 2);
    assert!(v["result"]["sources"].as_u64().unwrap() >= 3);
}

#[test]
fn concat_to_stdout() {
    let out = srcmap()
        .args([
            "concat",
            fixture("simple.js.map").to_str().unwrap(),
            fixture("second.js.map").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    // Output should be valid JSON source map
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], 3);
    assert!(v["mappings"].is_string());
}

#[test]
fn concat_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.js.map");
    let out = srcmap()
        .current_dir(dir.path())
        .args([
            "concat",
            fixture("simple.js.map").to_str().unwrap(),
            fixture("second.js.map").to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let content = fs::read_to_string(&out_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(v["version"], 3);
}

// ── remap ────────────────────────────────────────────────────────

#[test]
fn remap_dry_run() {
    let out = srcmap()
        .args(["remap", fixture("simple.js.map").to_str().unwrap(), "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["dryRun"], true);
}

// ── scopes ───────────────────────────────────────────────────────

#[test]
fn scopes_no_data() {
    let out =
        srcmap().args(["scopes", fixture("simple.js.map").to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("does not contain scopes"));
}

#[test]
fn scopes_no_data_json() {
    let out = srcmap()
        .args(["scopes", fixture("simple.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["code"], "NOT_FOUND");
}

// ── symbolicate ──────────────────────────────────────────────────

#[test]
fn symbolicate_json() {
    let out = srcmap()
        .args([
            "symbolicate",
            fixture("stacktrace.txt").to_str().unwrap(),
            "--map",
            &format!("simple.js={}", fixture("simple.js.map").display()),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["message"].as_str().unwrap().contains("something went wrong"));
    assert!(!v["frames"].as_array().unwrap().is_empty());
}

#[test]
fn symbolicate_from_stdin() {
    let trace = std::fs::read(fixture("stacktrace.txt")).unwrap();
    let out = srcmap()
        .args([
            "symbolicate",
            "-",
            "--map",
            &format!("simple.js={}", fixture("simple.js.map").display()),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(&trace).unwrap();
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("src/app.ts"));
}

// ── scopes with data ─────────────────────────────────────────────

#[test]
fn scopes_human() {
    let out =
        srcmap().args(["scopes", fixture("scoped.js.map").to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Original scopes"));
    assert!(stdout.contains("math.ts"));
    assert!(stdout.contains("module"));
    assert!(stdout.contains("Generated ranges"));
    assert!(stdout.contains("binding: _a"));
}

#[test]
fn scopes_json() {
    let out = srcmap()
        .args(["scopes", fixture("scoped.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!v["originalScopes"].as_array().unwrap().is_empty());
    assert!(!v["generatedRanges"].as_array().unwrap().is_empty());
    // Check original scope structure
    let scope = &v["originalScopes"][0]["scope"];
    assert_eq!(scope["kind"], "module");
    assert!(scope["variables"].as_array().unwrap().contains(&serde_json::json!("result")));
    // Check generated range bindings
    let range = &v["generatedRanges"][0];
    assert_eq!(range["definition"], 0);
    let child = &range["children"][0];
    assert_eq!(child["definition"], 1);
    assert!(child["isStackFrame"].as_bool().unwrap());
    assert_eq!(child["callSite"]["source"], "math.ts");
}

// ── schema ───────────────────────────────────────────────────────

#[test]
fn schema_output() {
    let out = srcmap().arg("schema").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"], "srcmap");
    let commands = v["commands"].as_array().unwrap();
    let names: Vec<&str> = commands.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"info"));
    assert!(names.contains(&"validate"));
    assert!(names.contains(&"lookup"));
    assert!(names.contains(&"resolve"));
    assert!(names.contains(&"decode"));
    assert!(names.contains(&"encode"));
    assert!(names.contains(&"mappings"));
    assert!(names.contains(&"concat"));
    assert!(names.contains(&"remap"));
    assert!(names.contains(&"symbolicate"));
    assert!(names.contains(&"scopes"));
    assert!(names.contains(&"schema"));
}

// ── sources ──────────────────────────────────────────────────

#[test]
fn sources_list_human() {
    let out =
        srcmap().args(["sources", fixture("simple.js.map").to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("src/app.ts"));
    assert!(stdout.contains("src/util.ts"));
    assert!(stdout.contains("no content"));
}

#[test]
fn sources_list_json() {
    let out = srcmap()
        .args(["sources", fixture("simple.js.map").to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["total"], 2);
    assert_eq!(v["withContent"], 1);
    let sources = v["sources"].as_array().unwrap();
    assert_eq!(sources[0]["source"], "src/app.ts");
    assert_eq!(sources[0]["hasContent"], true);
    assert_eq!(sources[1]["source"], "src/util.ts");
    assert_eq!(sources[1]["hasContent"], false);
}

#[test]
fn sources_extract() {
    let dir = tempfile::tempdir().unwrap();
    let out = srcmap()
        .current_dir(dir.path())
        .args([
            "sources",
            fixture("simple.js.map").to_str().unwrap(),
            "--extract",
            "-o",
            dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["extracted"].as_array().unwrap().len(), 1);
    assert_eq!(v["skipped"].as_array().unwrap().len(), 1);

    // Verify the extracted file exists and has correct content
    let extracted_path = dir.path().join("src/app.ts");
    assert!(extracted_path.exists());
    let content = fs::read_to_string(&extracted_path).unwrap();
    assert!(content.contains("const greet"));
}

// ── lookup with context ─────────────────────────────────────

#[test]
fn lookup_with_context_human() {
    let out = srcmap()
        .args(["lookup", fixture("simple.js.map").to_str().unwrap(), "0", "0", "--context", "1"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("src/app.ts"));
    assert!(stdout.contains("const greet"));
}

#[test]
fn lookup_with_context_json() {
    let out = srcmap()
        .args([
            "lookup",
            fixture("simple.js.map").to_str().unwrap(),
            "0",
            "0",
            "--context",
            "2",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["source"], "src/app.ts");
    assert!(v["context"].is_array());
    let ctx = v["context"].as_array().unwrap();
    assert!(!ctx.is_empty());
    // First line should be highlighted (line 0)
    assert_eq!(ctx[0]["highlight"], true);
    assert!(ctx[0]["text"].as_str().unwrap().contains("const greet"));
}

// ── lookup context edge cases ────────────────────────────────

#[test]
fn lookup_context_zero_no_context_block() {
    // --context 0 should behave like normal lookup (no context in output)
    let out = srcmap()
        .args([
            "lookup",
            fixture("simple.js.map").to_str().unwrap(),
            "0",
            "0",
            "--context",
            "0",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["source"], "src/app.ts");
    assert!(v["context"].is_null());
}

#[test]
fn lookup_context_no_sources_content() {
    // second.js.map has no sourcesContent — context should be absent, not crash
    let out = srcmap()
        .args([
            "lookup",
            fixture("second.js.map").to_str().unwrap(),
            "0",
            "0",
            "--context",
            "3",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["source"], "src/other.ts");
    assert!(v["context"].is_null());
}

// ── sources extract with special paths ──────────────────────

#[test]
fn sources_extract_webpack_paths() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    let outside_dir = dir.path().join("lib");
    fs::create_dir(&output_dir).unwrap();
    fs::create_dir(&outside_dir).unwrap();
    let outside_sentinel = outside_dir.join("external.ts");
    fs::write(&outside_sentinel, "sentinel").unwrap();
    let out = srcmap()
        .current_dir(dir.path())
        .args([
            "sources",
            fixture("webpack.js.map").to_str().unwrap(),
            "--extract",
            "-o",
            output_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["extracted"].as_array().unwrap().len(), 3);

    // webpack:///src/index.ts → src/index.ts
    let index_path = output_dir.join("src/index.ts");
    assert!(index_path.exists());
    let content = fs::read_to_string(&index_path).unwrap();
    assert!(content.contains("export const init"));

    // webpack:///./src/helpers/utils.ts → src/helpers/utils.ts
    let utils_path = output_dir.join("src/helpers/utils.ts");
    assert!(utils_path.exists());

    // ../lib/external.ts → lib/external.ts (leading ../ stripped)
    let ext_path = output_dir.join("lib/external.ts");
    assert!(ext_path.exists());
    assert_eq!(fs::read_to_string(outside_sentinel).unwrap(), "sentinel");
}

#[test]
fn sources_extract_security_skips_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    fs::create_dir(&output_dir).unwrap();
    let sentinel = output_dir.join("existing.ts");
    fs::write(&sentinel, "sentinel").unwrap();
    let map = write_sources_map(dir.path(), &["existing.ts"]);

    let result = extract_sources(&map, &output_dir);

    assert_eq!(fs::read_to_string(sentinel).unwrap(), "sentinel");
    assert_eq!(result["extracted"], serde_json::json!([]));
    assert_eq!(result["skipped"], serde_json::json!(["existing.ts"]));
}

#[cfg(unix)]
#[test]
fn sources_extract_security_rejects_symlink_output_root() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside_dir = root.join("outside");
    let output_dir = root.join("output");
    fs::create_dir(&outside_dir).unwrap();
    symlink(&outside_dir, &output_dir).unwrap();
    let map = write_sources_map(dir.path(), &["source.ts"]);

    let out = srcmap()
        .args([
            "sources",
            map.to_str().unwrap(),
            "--extract",
            "-o",
            output_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(!outside_dir.join("source.ts").exists());
}

#[cfg(unix)]
#[test]
fn sources_extract_security_rejects_symlink_output_ancestor() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside_dir = root.join("outside");
    let linked_dir = root.join("linked");
    let output_dir = linked_dir.join("nested");
    fs::create_dir(&outside_dir).unwrap();
    symlink(&outside_dir, &linked_dir).unwrap();
    let map = write_sources_map(&root, &["source.ts"]);

    let out = srcmap()
        .args([
            "sources",
            map.to_str().unwrap(),
            "--extract",
            "-o",
            output_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(!outside_dir.join("nested/source.ts").exists());
}

#[cfg(unix)]
#[test]
fn sources_extract_security_skips_parent_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    let outside_dir = dir.path().join("outside");
    fs::create_dir(&output_dir).unwrap();
    fs::create_dir(&outside_dir).unwrap();
    let sentinel = outside_dir.join("target.ts");
    fs::write(&sentinel, "sentinel").unwrap();
    symlink(&outside_dir, output_dir.join("linked")).unwrap();
    let map = write_sources_map(dir.path(), &["linked/target.ts"]);

    let result = extract_sources(&map, &output_dir);

    assert_eq!(fs::read_to_string(sentinel).unwrap(), "sentinel");
    assert_eq!(result["extracted"], serde_json::json!([]));
    assert_eq!(result["skipped"], serde_json::json!(["linked/target.ts"]));
}

#[cfg(unix)]
#[test]
fn sources_extract_security_skips_final_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    fs::create_dir(&output_dir).unwrap();
    let sentinel = dir.path().join("outside.ts");
    fs::write(&sentinel, "sentinel").unwrap();
    symlink(&sentinel, output_dir.join("linked.ts")).unwrap();
    let map = write_sources_map(dir.path(), &["linked.ts"]);

    let result = extract_sources(&map, &output_dir);

    assert_eq!(fs::read_to_string(sentinel).unwrap(), "sentinel");
    assert_eq!(result["extracted"], serde_json::json!([]));
    assert_eq!(result["skipped"], serde_json::json!(["linked.ts"]));
}

#[test]
fn sources_extract_security_skips_absolute_and_backslash_paths() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    fs::create_dir(&output_dir).unwrap();
    let sources = [
        "/absolute.ts",
        "C:/outside.ts",
        "C:outside.ts",
        r"C:\outside.ts",
        r"\\server\share\outside.ts",
        r"..\outside.ts",
    ];
    let map = write_sources_map(dir.path(), &sources);

    let result = extract_sources(&map, &output_dir);

    assert_eq!(result["extracted"], serde_json::json!([]));
    assert_eq!(result["skipped"], serde_json::json!(sources));
    assert_eq!(fs::read_dir(&output_dir).unwrap().count(), 0);
}

#[test]
fn sources_extract_security_reports_output_file_as_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("not-a-directory");
    fs::write(&output_file, "sentinel").unwrap();
    let map = write_sources_map(dir.path(), &["source.ts"]);

    let out = srcmap()
        .args([
            "sources",
            map.to_str().unwrap(),
            "--extract",
            "-o",
            output_file.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let error: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(error["code"], "IO_ERROR");
    assert_eq!(fs::read_to_string(output_file).unwrap(), "sentinel");
}

#[test]
fn sources_extract_security_human_skip_message_is_neutral() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    fs::create_dir(&output_dir).unwrap();
    let map = write_sources_map(dir.path(), &["C:outside.ts"]);

    let out = srcmap()
        .args(["sources", map.to_str().unwrap(), "--extract", "-o", output_dir.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Skipped 1 source"));
    assert!(!stdout.contains("without content"));
}

#[cfg(unix)]
#[test]
fn sources_extract_security_resists_parent_swap_race() {
    use std::os::unix::fs::symlink;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const RACE_SOURCES: usize = 4_096;

    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("output");
    let raced_dir = output_dir.join("raced");
    let parked_dir = output_dir.join("parked");
    let outside_dir = dir.path().join("outside");
    fs::create_dir(&output_dir).unwrap();
    fs::create_dir(&raced_dir).unwrap();
    fs::create_dir(&outside_dir).unwrap();
    let sources: Vec<String> =
        (0..RACE_SOURCES).map(|index| format!("raced/source-{index}.ts")).collect();
    let contents = vec!["content"; sources.len()];
    let map_path = dir.path().join("race.js.map");
    let map = serde_json::json!({
        "version": 3,
        "file": "race.js",
        "sources": sources,
        "names": [],
        "mappings": "",
        "sourcesContent": contents,
    });
    fs::write(&map_path, serde_json::to_vec(&map).unwrap()).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let attacker_stop = Arc::clone(&stop);
    let attacker_raced = raced_dir;
    let attacker_parked = parked_dir;
    let attacker_outside = outside_dir.clone();
    let attacker = thread::spawn(move || {
        while !attacker_stop.load(Ordering::Relaxed) {
            if fs::rename(&attacker_raced, &attacker_parked).is_ok() {
                if symlink(&attacker_outside, &attacker_raced).is_ok() {
                    thread::yield_now();
                    let _ = fs::remove_file(&attacker_raced);
                }
                if fs::rename(&attacker_parked, &attacker_raced).is_err() {
                    let _ = fs::remove_dir_all(&attacker_raced);
                    let _ = fs::rename(&attacker_parked, &attacker_raced);
                }
            }
            thread::yield_now();
        }
    });

    let out = srcmap()
        .args([
            "sources",
            map_path.to_str().unwrap(),
            "--extract",
            "-o",
            output_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    attacker.join().unwrap();

    if !out.status.success() {
        let error: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(error["code"], "IO_ERROR");
    }
    assert_eq!(fs::read_dir(outside_dir).unwrap().count(), 0);
}

// ── fetch ────────────────────────────────────────────────────

#[test]
fn fetch_invalid_url() {
    let out = srcmap().args(["fetch", "not-a-url"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("INVALID_INPUT") || stderr.contains("http://"));
}

#[test]
fn fetch_invalid_url_json() {
    let out = srcmap().args(["fetch", "not-a-url", "--json"]).output().unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["code"], "INVALID_INPUT");
}

fn read_request(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut request_line = Vec::with_capacity(128);
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).unwrap();
        request_line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
        assert!(request_line.len() < 8_192, "HTTP request line is unexpectedly long");
    }
    let request_line = String::from_utf8_lossy(&request_line);
    request_line.split_whitespace().nth(1).unwrap_or_default().to_string()
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        write!(response, "{name}: {value}\r\n").unwrap();
    }
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(body);
    response
}

fn accept_connection(listener: &TcpListener) -> TcpStream {
    const ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);

    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "timed out waiting for local HTTP request");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("failed to accept local HTTP request: {error}"),
        }
    }
}

fn serve_once(listener: TcpListener, reply: String, delay: Duration) -> Receiver<()> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = sender.send(());
        let _ = read_request(&mut stream);
        thread::sleep(delay);
        let _ = stream.write_all(reply.as_bytes());
    });
    receiver
}

fn local_url(listener: &TcpListener, path: &str) -> String {
    format!("http://{}{path}", listener.local_addr().unwrap())
}

fn source_map_json() -> &'static str {
    r#"{"version":3,"sources":[],"names":[],"mappings":""}"#
}

#[test]
fn fetch_security_rejects_existing_bundle_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let reply = response("200 OK", &[], "console.log(1)");
    let received = serve_once(listener.try_clone().unwrap(), reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();
    let bundle_path = output.path().join("bundle.js");
    fs::write(&bundle_path, "sentinel").unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(bundle_path).unwrap(), "sentinel");
}

#[test]
fn fetch_security_rejects_existing_source_map_destination() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for body in ["console.log(1);\n//# sourceMappingURL=bundle.js.map", source_map_json()] {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(response("200 OK", &[], body).as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();
    let map_path = output.path().join("bundle.js.map");
    fs::write(&map_path, "sentinel").unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(map_path).unwrap(), "sentinel");
}

#[cfg(unix)]
#[test]
fn fetch_security_rejects_symlink_bundle_destination() {
    use std::os::unix::fs::symlink;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let reply = response("200 OK", &[], "console.log(1)");
    let received = serve_once(listener.try_clone().unwrap(), reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "sentinel").unwrap();
    symlink(outside.path(), output.path().join("bundle.js")).unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "sentinel");
}

#[cfg(unix)]
#[test]
fn fetch_security_rejects_symlink_output_root() {
    use std::os::unix::fs::symlink;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let inline = "console.log(1);\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6IiJ9";
    let reply = response("200 OK", &[], inline);
    let received = serve_once(listener.try_clone().unwrap(), reply, Duration::ZERO);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside_dir = root.join("outside");
    let output_dir = root.join("output");
    fs::create_dir(&outside_dir).unwrap();
    symlink(&outside_dir, &output_dir).unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(!out.status.success());
    assert!(!outside_dir.join("bundle.js").exists());
}

#[cfg(unix)]
#[test]
fn fetch_security_rejects_symlink_output_ancestor() {
    use std::os::unix::fs::symlink;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let inline = "console.log(1);\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6IiJ9";
    let reply = response("200 OK", &[], inline);
    let received = serve_once(listener.try_clone().unwrap(), reply, Duration::ZERO);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside_dir = root.join("outside");
    let linked_dir = root.join("linked");
    let output_dir = linked_dir.join("nested");
    fs::create_dir(&outside_dir).unwrap();
    symlink(&outside_dir, &linked_dir).unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(!out.status.success());
    assert!(!outside_dir.join("nested/bundle.js").exists());
}

#[cfg(unix)]
#[test]
fn fetch_security_rejects_symlink_source_map_destination() {
    use std::os::unix::fs::symlink;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for body in ["console.log(1);\n//# sourceMappingURL=bundle.js.map", source_map_json()] {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(response("200 OK", &[], body).as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "sentinel").unwrap();
    symlink(outside.path(), output.path().join("bundle.js.map")).unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(outside.path()).unwrap(), "sentinel");
}

#[test]
fn fetch_security_rejects_bundle_and_map_filename_collisions() {
    for map_ref in ["?map", "#map", "bundle.js"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = listener.try_clone().unwrap();
        let bundle = format!("console.log(1);\n//# sourceMappingURL={map_ref}");
        let expected_bundle = bundle.clone();
        let received = thread::spawn(move || {
            for body in [bundle.as_str(), source_map_json()] {
                let mut stream = accept_connection(&server);
                let _ = read_request(&mut stream);
                stream.write_all(response("200 OK", &[], body).as_bytes()).unwrap();
            }
        });
        let output = tempfile::tempdir().unwrap();

        let out = srcmap()
            .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
            .arg(output.path())
            .output()
            .unwrap();

        received.join().unwrap();
        assert!(!out.status.success(), "accepted colliding map reference {map_ref:?}");
        assert_eq!(fs::read_to_string(output.path().join("bundle.js")).unwrap(), expected_bundle);
    }
}

#[test]
fn fetch_conventional_source_map_ignores_not_found() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for reply in
            [response("200 OK", &[], "console.log(1)"), response("404 Not Found", &[], "missing")]
        {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "--json", "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(result["sourceMap"].is_null());
}

#[test]
fn fetch_conventional_source_map_propagates_server_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for reply in [
            response("200 OK", &[], "console.log(1)"),
            response("500 Internal Server Error", &[], "failed"),
        ] {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("HTTP 500"));
}

#[test]
fn fetch_conventional_source_map_propagates_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        let mut bundle_stream = accept_connection(&server);
        let _ = read_request(&mut bundle_stream);
        bundle_stream.write_all(response("200 OK", &[], "console.log(1)").as_bytes()).unwrap();

        let mut map_stream = accept_connection(&server);
        let _ = read_request(&mut map_stream);
        thread::sleep(Duration::from_millis(150));
        let _ = map_stream.write_all(response("200 OK", &[], source_map_json()).as_bytes());
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .env("SRCMAP_FETCH_TIMEOUT_MS", "50")
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
}

#[test]
fn fetch_conventional_source_map_propagates_cross_origin_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_url = local_url(&target, "/bundle.js.map");
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for reply in [
            response("200 OK", &[], "console.log(1)"),
            response("302 Found", &[("Location", &target_url)], ""),
        ] {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let target_received =
        serve_once(target, response("200 OK", &[], source_map_json()), Duration::ZERO);
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cross-origin"));
    assert!(target_received.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn fetch_conventional_source_map_propagates_body_read_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        let mut bundle_stream = accept_connection(&server);
        let _ = read_request(&mut bundle_stream);
        bundle_stream.write_all(response("200 OK", &[], "console.log(1)").as_bytes()).unwrap();

        let mut map_stream = accept_connection(&server);
        let _ = read_request(&mut map_stream);
        map_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
            .unwrap();
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    received.join().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("failed to fetch"));
}

#[test]
fn fetch_security_blocks_cross_origin_source_map_without_opt_in() {
    let bundle_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let map_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let map_url = local_url(&map_listener, "/bundle.js.map");
    let bundle = format!("console.log(1);\n//# sourceMappingURL={map_url}");
    let bundle_reply = response("200 OK", &[], &bundle);
    let bundle_received =
        serve_once(bundle_listener.try_clone().unwrap(), bundle_reply, Duration::ZERO);
    let map_reply = response("200 OK", &[], source_map_json());
    let map_received = serve_once(map_listener, map_reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&bundle_listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    bundle_received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cross-origin"));
    assert!(map_received.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn fetch_security_allows_cross_origin_source_map_with_opt_in() {
    let bundle_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let map_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let map_url = local_url(&map_listener, "/bundle.js.map");
    let bundle = format!("console.log(1);\n//# sourceMappingURL={map_url}");
    let bundle_reply = response("200 OK", &[], &bundle);
    serve_once(bundle_listener.try_clone().unwrap(), bundle_reply, Duration::ZERO);
    let map_reply = response("200 OK", &[], source_map_json());
    let map_received = serve_once(map_listener, map_reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&bundle_listener, "/bundle.js"), "--allow-cross-origin", "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    map_received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(output.path().join("bundle.js.map").is_file());
}

#[test]
fn fetch_security_blocks_cross_origin_redirect_without_request() {
    let redirect_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_url = local_url(&target_listener, "/bundle.js");
    let redirect_reply = response("302 Found", &[("Location", &target_url)], "");
    serve_once(redirect_listener.try_clone().unwrap(), redirect_reply, Duration::ZERO);
    let target_reply = response("200 OK", &[], "console.log(1)");
    let target_received = serve_once(target_listener, target_reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&redirect_listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cross-origin"));
    assert!(target_received.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn fetch_security_allows_cross_origin_redirect_with_opt_in() {
    let redirect_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_url = local_url(&target_listener, "/bundle.js");
    let redirect_reply = response("302 Found", &[("Location", &target_url)], "");
    serve_once(redirect_listener.try_clone().unwrap(), redirect_reply, Duration::ZERO);
    let inline = "console.log(1);\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6IiJ9";
    let target_reply = response("200 OK", &[], inline);
    let target_received = serve_once(target_listener, target_reply, Duration::ZERO);
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&redirect_listener, "/bundle.js"), "--allow-cross-origin", "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    target_received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(output.path().join("bundle.js").is_file());
    assert!(output.path().join("bundle.js.map").is_file());
}

#[test]
fn fetch_security_allows_same_origin_relative_source_map() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for body in ["console.log(1);\n//# sourceMappingURL=bundle.js.map", source_map_json()] {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            stream.write_all(response("200 OK", &[], body).as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    received.join().unwrap();
    assert!(output.path().join("bundle.js.map").is_file());
}

#[test]
fn fetch_security_uses_final_redirect_url_for_relative_map_and_filename() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        let replies = [
            response("302 Found", &[("Location", "/assets/final.js")], ""),
            response("200 OK", &[], "console.log(1);\n//# sourceMappingURL=final.js.map"),
            response("200 OK", &[], source_map_json()),
        ];
        let expected_paths = ["/old/bundle.js", "/assets/final.js", "/assets/final.js.map"];

        for (reply, expected_path) in replies.iter().zip(expected_paths) {
            let mut stream = accept_connection(&server);
            assert_eq!(read_request(&mut stream), expected_path);
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/old/bundle.js"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    received.join().unwrap();
    assert!(output.path().join("final.js").is_file());
    assert!(output.path().join("final.js.map").is_file());
    assert!(!output.path().join("bundle.js").exists());
}

#[test]
fn fetch_security_bounds_redirects() {
    const EXPECTED_REQUESTS: usize = 11;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let received = thread::spawn(move || {
        for _ in 0..EXPECTED_REQUESTS {
            let mut stream = accept_connection(&server);
            let _ = read_request(&mut stream);
            let reply = response("302 Found", &[("Location", "/loop")], "");
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    let output = tempfile::tempdir().unwrap();

    let out = srcmap()
        .args(["fetch", &local_url(&listener, "/loop"), "-o"])
        .arg(output.path())
        .output()
        .unwrap();

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("too many redirects"));
    received.join().unwrap();
}

#[test]
fn fetch_security_times_out_slow_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let inline = "console.log(1);\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6IiJ9";
    let slow_reply = response("200 OK", &[], inline);
    let accepted =
        serve_once(listener.try_clone().unwrap(), slow_reply, Duration::from_millis(1_500));
    let output = tempfile::tempdir().unwrap();
    let url = local_url(&listener, "/slow.js");
    let output_path = output.path().to_path_buf();
    let fetch = thread::spawn(move || {
        srcmap()
            .env("SRCMAP_FETCH_TIMEOUT_MS", "50")
            .args(["fetch", &url, "-o"])
            .arg(output_path)
            .output()
            .unwrap()
    });
    accepted.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    let out = fetch.join().unwrap();

    let elapsed = started.elapsed();
    assert!(!out.status.success());
    assert!(
        elapsed < Duration::from_secs(1),
        "fetch took {elapsed:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
}

#[test]
fn fetch_security_uses_one_deadline_across_redirects() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.try_clone().unwrap();
    let (accepted_sender, accepted_receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let received = thread::spawn(move || {
        server.set_nonblocking(true).unwrap();
        let replies = [
            (response("302 Found", &[("Location", "/two")], ""), Duration::from_millis(350)),
            (response("302 Found", &[("Location", "/final.js")], ""), Duration::from_millis(350)),
            (
                response(
                    "200 OK",
                    &[],
                    "console.log(1);\n//# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbXSwibmFtZXMiOltdLCJtYXBwaW5ncyI6IiJ9",
                ),
                Duration::from_millis(100),
            ),
        ];
        for (index, (reply, delay)) in replies.into_iter().enumerate() {
            let mut stream = loop {
                match server.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        break stream;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if server_stop.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("failed to accept redirect request: {error}"),
                }
            };
            if index == 0 {
                accepted_sender.send(()).unwrap();
            }
            let _ = read_request(&mut stream);
            thread::sleep(delay);
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    let output = tempfile::tempdir().unwrap();
    let url = local_url(&listener, "/one");
    let output_path = output.path().to_path_buf();
    let fetch = thread::spawn(move || {
        srcmap()
            .env("SRCMAP_FETCH_TIMEOUT_MS", "500")
            .args(["fetch", &url, "-o"])
            .arg(output_path)
            .output()
            .unwrap()
    });
    accepted_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    let out = fetch.join().unwrap();
    let elapsed = started.elapsed();

    stop.store(true, Ordering::Relaxed);
    received.join().unwrap();
    assert!(!out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        elapsed < Duration::from_millis(900),
        "redirect chain took {elapsed:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
}

// ── schema includes new commands ─────────────────────────────

#[test]
fn schema_includes_new_commands() {
    let out = srcmap().arg("schema").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let commands = v["commands"].as_array().unwrap();
    let names: Vec<&str> = commands.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"fetch"));
    assert!(names.contains(&"sources"));

    let fetch = commands.iter().find(|command| command["name"] == "fetch").unwrap();
    assert_eq!(fetch["flags"]["--allow-cross-origin"]["type"], "bool");
    assert_eq!(fetch["flags"]["--allow-cross-origin"]["default"], false);
}

// ── error handling ───────────────────────────────────────────────

#[test]
fn missing_file_error() {
    let out = srcmap().args(["info", "/nonexistent/path/to/file.map"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("error:"));
}

#[test]
fn missing_file_json_error() {
    let out = srcmap().args(["info", "/nonexistent/path/to/file.map", "--json"]).output().unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["code"], "IO_ERROR");
}

// ── stdin support ────────────────────────────────────────────────

#[test]
fn info_from_stdin() {
    let map_content = fs::read(fixture("simple.js.map")).unwrap();
    let out = srcmap()
        .args(["info", "-", "--json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(&map_content).unwrap();
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["sources"], 2);
}

#[test]
fn decode_from_stdin() {
    let out = srcmap()
        .arg("decode")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(b"AAAA").unwrap();
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0][0], serde_json::json!([0, 0, 0, 0]));
}
