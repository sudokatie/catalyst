//! Integration tests for the build system

use std::fs;
use std::io::Write;
use tempfile::TempDir;

use catalyst::{expand_target, Config, Label, QueryEngine, Resolver};

fn create_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    // Create WORKSPACE file
    fs::write(dir.path().join("WORKSPACE"), "workspace(name = \"test\")\n").unwrap();
    dir
}

fn write_build_file(dir: &std::path::Path, package: &str, content: &str) {
    let pkg_dir = dir.join(package);
    fs::create_dir_all(&pkg_dir).unwrap();
    let build_path = pkg_dir.join("BUILD");
    let mut file = fs::File::create(build_path).unwrap();
    write!(file, "{}", content).unwrap();
}

#[test]
fn build_rust_binary_from_scratch() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "hello",
    srcs = ["main.rs"],
)
"#,
    );

    // Create source file
    let app_dir = ws.path().join("app");
    fs::write(app_dir.join("main.rs"), "fn main() { println!(\"Hello\"); }").unwrap();

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("app", "hello");
    let node_id = resolver.resolve(&label).unwrap();

    // Verify target was resolved
    assert!(resolver.get_target(&label).is_some());

    let target = resolver.get_target(&label).unwrap();
    assert_eq!(target.rule_type, "rust_binary");
    assert_eq!(target.srcs.len(), 1);

    // Expand to actions
    let output_dir = ws.path().join(".catalyst/out");
    let expansion = expand_target(target, &output_dir);

    assert_eq!(expansion.actions.len(), 1);
    assert!(expansion.actions[0].command.contains(&"rustc".to_string()));
}

#[test]
fn build_with_dependencies() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "lib",
        r#"
rust_library(
    name = "mylib",
    srcs = ["lib.rs"],
)
"#,
    );

    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "myapp",
    srcs = ["main.rs"],
    deps = ["//lib:mylib"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("app", "myapp");
    resolver.resolve(&label).unwrap();

    // Both targets should be resolved
    assert!(resolver.get_target(&Label::new("app", "myapp")).is_some());
    assert!(resolver.get_target(&Label::new("lib", "mylib")).is_some());

    // Check dependency graph
    let query = QueryEngine::new(resolver.graph());
    let app_id = query
        .filter_targets(|l| l.name == "myapp")
        .into_iter()
        .next()
        .unwrap();

    let deps = query.transitive_deps(app_id);
    assert_eq!(deps.len(), 1);
}

#[test]
fn query_deps_works() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "base",
        r#"
rust_library(
    name = "base",
)
"#,
    );

    write_build_file(
        ws.path(),
        "mid",
        r#"
rust_library(
    name = "mid",
    deps = ["//base:base"],
)
"#,
    );

    write_build_file(
        ws.path(),
        "top",
        r#"
rust_binary(
    name = "top",
    deps = ["//mid:mid"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    resolver.resolve(&Label::new("top", "top")).unwrap();

    let query = QueryEngine::new(resolver.graph());
    let top_id = query
        .filter_targets(|l| l.name == "top")
        .into_iter()
        .next()
        .unwrap();

    // Transitive deps should include both mid and base
    let deps = query.transitive_deps(top_id);
    assert_eq!(deps.len(), 2);

    // Topo order should be base, mid, top
    let order = query.topo_order().unwrap();
    assert_eq!(order.len(), 3);
}

#[test]
fn config_loads_defaults() {
    let ws = create_workspace();
    let config = Config::load_default(Some(ws.path())).unwrap();

    assert!(config.jobs() > 0);
    assert!(config.build.sandbox);
}

#[test]
fn genrule_expands_correctly() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "gen",
        r#"
genrule(
    name = "generate",
    srcs = ["input.txt"],
    outs = ["output.txt"],
    cmd = "cat $< > $@",
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("gen", "generate");
    resolver.resolve(&label).unwrap();

    let target = resolver.get_target(&label).unwrap();
    assert_eq!(target.rule_type, "genrule");

    let output_dir = ws.path().join(".catalyst/out");
    let expansion = expand_target(target, &output_dir);

    assert_eq!(expansion.actions.len(), 1);
    let cmd = &expansion.actions[0].command;
    assert_eq!(cmd[0], "sh");
    assert_eq!(cmd[1], "-c");
}

#[test]
fn filegroup_has_no_actions() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "files",
        r#"
filegroup(
    name = "srcs",
    srcs = ["a.txt", "b.txt"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("files", "srcs");
    resolver.resolve(&label).unwrap();

    let target = resolver.get_target(&label).unwrap();
    let output_dir = ws.path().join(".catalyst/out");
    let expansion = expand_target(target, &output_dir);

    // Filegroup should produce no actions
    assert!(expansion.actions.is_empty());
}

#[test]
fn cycle_detection_works() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "a",
        r#"
rust_library(
    name = "a",
    deps = ["//b:b"],
)
"#,
    );

    write_build_file(
        ws.path(),
        "b",
        r#"
rust_library(
    name = "b",
    deps = ["//a:a"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let result = resolver.resolve(&Label::new("a", "a"));

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("cycle") || err.to_string().contains("Cycle"));
}

#[test]
fn parallel_build_of_independent_targets() {
    let ws = create_workspace();

    // Create 4 independent libraries
    for i in 1..=4 {
        write_build_file(
            ws.path(),
            &format!("lib{}", i),
            &format!(
                r#"
rust_library(
    name = "lib{}",
)
"#,
                i
            ),
        );
    }

    // Create app that depends on all 4
    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "app",
    deps = ["//lib1:lib1", "//lib2:lib2", "//lib3:lib3", "//lib4:lib4"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    resolver.resolve(&Label::new("app", "app")).unwrap();

    let query = QueryEngine::new(resolver.graph());
    let order = query.topo_order().unwrap();

    // Should have 5 targets total
    assert_eq!(order.len(), 5);

    // App should be last
    let app_id = query
        .filter_targets(|l| l.name == "app")
        .into_iter()
        .next()
        .unwrap();

    let app_pos = order.iter().position(|&id| id == app_id).unwrap();
    assert_eq!(app_pos, 4); // Last position
}
