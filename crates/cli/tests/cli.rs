use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn srcmap() -> Command {
    Command::new(env!("CARGO_BIN_EXE_srcmap"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
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
    let out = srcmap()
        .current_dir(dir.path())
        .args([
            "sources",
            fixture("webpack.js.map").to_str().unwrap(),
            "--extract",
            "-o",
            dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["extracted"].as_array().unwrap().len(), 3);

    // webpack:///src/index.ts → src/index.ts
    let index_path = dir.path().join("src/index.ts");
    assert!(index_path.exists());
    let content = fs::read_to_string(&index_path).unwrap();
    assert!(content.contains("export const init"));

    // webpack:///./src/helpers/utils.ts → src/helpers/utils.ts
    let utils_path = dir.path().join("src/helpers/utils.ts");
    assert!(utils_path.exists());

    // ../lib/external.ts → lib/external.ts (leading ../ stripped)
    let ext_path = dir.path().join("lib/external.ts");
    assert!(ext_path.exists());
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
