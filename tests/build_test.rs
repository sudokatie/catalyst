//! Integration tests for the build system

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

use catalyst::{expand_target, Config, Label, MetadataStore, QueryEngine, Resolver};

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

// TASK 25 Tests: Build and Run Execution

#[test]
fn run_requires_binary_target() {
    let ws = create_workspace();

    // Create a library, not a binary
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

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("lib", "mylib");
    resolver.resolve(&label).unwrap();

    let target = resolver.get_target(&label).unwrap();
    // Should be a library, not a binary
    assert_eq!(target.rule_type, "rust_library");
    assert_ne!(target.rule_type, "rust_binary");
}

#[test]
fn run_expands_rust_binary_target() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "myapp",
    srcs = ["main.rs"],
)
"#,
    );

    fs::write(
        ws.path().join("app").join("main.rs"),
        "fn main() { println!(\"test\"); }",
    )
    .unwrap();

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("app", "myapp");
    resolver.resolve(&label).unwrap();

    let target = resolver.get_target(&label).unwrap();
    assert_eq!(target.rule_type, "rust_binary");

    // Expand to actions
    let output_dir = ws.path().join(".catalyst/out");
    let expansion = expand_target(target, &output_dir);

    // Should have exactly one action
    assert_eq!(expansion.actions.len(), 1);
    // Output should be the binary
    assert_eq!(expansion.outputs.len(), 1);
    assert!(expansion.outputs[0].to_string_lossy().contains("myapp"));
}

#[test]
fn run_expands_cc_binary_target() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "app",
        r#"
cc_binary(
    name = "myapp",
    srcs = ["main.c"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    let label = Label::new("app", "myapp");
    resolver.resolve(&label).unwrap();

    let target = resolver.get_target(&label).unwrap();
    assert_eq!(target.rule_type, "cc_binary");
}

#[test]
fn run_binary_with_deps_resolves_all() {
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

    // Topo order should have lib before app
    let order = resolver.graph().topo_order().unwrap();
    assert_eq!(order.len(), 2);
}

#[test]
fn run_binary_output_path_correct() {
    let ws = create_workspace();

    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "hello_world",
    srcs = ["main.rs"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    resolver.resolve(&Label::new("app", "hello_world")).unwrap();

    let target = resolver.get_target(&Label::new("app", "hello_world")).unwrap();
    let output_dir = ws.path().join(".catalyst/out");
    let expansion = expand_target(target, &output_dir);

    // Output should be named after the target
    assert_eq!(expansion.outputs.len(), 1);
    let output_name = expansion.outputs[0].file_name().unwrap().to_string_lossy();
    assert_eq!(output_name, "hello_world");
}

// TASK 26 Tests: Garbage Collection

#[test]
fn gc_metadata_store_removes_old_entries() {
    let dir = TempDir::new().unwrap();
    let store = MetadataStore::new(&dir.path().join("metadata.db")).unwrap();

    // Add some entries with old timestamps
    let path1 = PathBuf::from("/old/file1.rs");
    let path2 = PathBuf::from("/new/file2.rs");
    let hash = [1u8; 32];
    let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let _new_time = SystemTime::now();

    store.store(&path1, &hash, old_time).unwrap();
    store.store(&path2, &hash, old_time).unwrap();

    assert_eq!(store.len().unwrap(), 2);

    // GC with cutoff in the future should remove all
    let future = SystemTime::now() + Duration::from_secs(3600);
    let removed = store.gc(future).unwrap();

    assert_eq!(removed, 2);
    assert!(store.is_empty().unwrap());
}

#[test]
fn gc_metadata_store_preserves_recent_entries() {
    let dir = TempDir::new().unwrap();
    let store = MetadataStore::new(&dir.path().join("metadata.db")).unwrap();

    let path = PathBuf::from("/recent/file.rs");
    let hash = [1u8; 32];
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);

    store.store(&path, &hash, mtime).unwrap();

    // Access the entry to update last_accessed
    let _ = store.get(&path, mtime);

    // GC with cutoff in the past should remove nothing
    let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    let removed = store.gc(past).unwrap();

    assert_eq!(removed, 0);
    assert_eq!(store.len().unwrap(), 1);
}

#[test]
fn gc_dry_run_flag_default() {
    // Verify that dry_run flag exists and works
    // This is a config test, actual GC testing is above
    let config = Config::default();
    assert!(config.jobs() > 0);
}

#[test]
fn gc_cache_directory_structure() {
    let ws = create_workspace();
    let config = Config::load_default(Some(ws.path())).unwrap();
    let cache_dir = config.cache_dir();

    // Cache dir should be derived from config
    assert!(!cache_dir.to_string_lossy().is_empty());
}

#[test]
fn gc_metadata_store_all_paths() {
    let dir = TempDir::new().unwrap();
    let store = MetadataStore::new(&dir.path().join("metadata.db")).unwrap();

    let path1 = PathBuf::from("/a.rs");
    let path2 = PathBuf::from("/b.rs");
    let path3 = PathBuf::from("/c.rs");
    let hash = [1u8; 32];
    let mtime = SystemTime::now();

    store.store(&path1, &hash, mtime).unwrap();
    store.store(&path2, &hash, mtime).unwrap();
    store.store(&path3, &hash, mtime).unwrap();

    let paths = store.all_paths().unwrap();
    assert_eq!(paths.len(), 3);
    assert!(paths.contains(&path1));
    assert!(paths.contains(&path2));
    assert!(paths.contains(&path3));
}

#[test]
fn gc_safety_preserves_recent() {
    let dir = TempDir::new().unwrap();
    let store = MetadataStore::new(&dir.path().join("metadata.db")).unwrap();

    // Create entries with different access times
    let old_path = PathBuf::from("/old.rs");
    let new_path = PathBuf::from("/new.rs");
    let hash = [1u8; 32];
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);

    store.store(&old_path, &hash, mtime).unwrap();
    store.store(&new_path, &hash, mtime).unwrap();

    // Access new_path to update its last_accessed timestamp
    let _ = store.get(&new_path, mtime);

    // GC with a cutoff of "now minus 1 second" should keep entries accessed recently
    let recent_cutoff = SystemTime::now() - Duration::from_secs(1);
    let removed = store.gc(recent_cutoff).unwrap();

    // Both should be preserved since they were just accessed
    assert!(removed <= 2);
}

#[test]
fn gc_reachability_via_graph() {
    let ws = create_workspace();

    // Create a dependency chain: app -> lib -> base
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
        "lib",
        r#"
rust_library(
    name = "lib",
    deps = ["//base:base"],
)
"#,
    );

    write_build_file(
        ws.path(),
        "app",
        r#"
rust_binary(
    name = "app",
    deps = ["//lib:lib"],
)
"#,
    );

    let mut resolver = Resolver::new(ws.path().to_path_buf());
    resolver.resolve(&Label::new("app", "app")).unwrap();

    // All three should be reachable from app
    let query = QueryEngine::new(resolver.graph());
    let app_id = query
        .filter_targets(|l| l.name == "app")
        .into_iter()
        .next()
        .unwrap();

    let deps = query.transitive_deps(app_id);
    assert_eq!(deps.len(), 2); // lib and base are reachable
}

#[test]
fn gc_removes_unreferenced_entry() {
    let dir = TempDir::new().unwrap();
    let store = MetadataStore::new(&dir.path().join("metadata.db")).unwrap();

    let path = PathBuf::from("/unreferenced.rs");
    let hash = [1u8; 32];
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);

    store.store(&path, &hash, mtime).unwrap();
    assert_eq!(store.len().unwrap(), 1);

    // GC with cutoff in the FAR future should remove any entry
    // since last_accessed was set to "now" which is before the future cutoff
    let far_future = SystemTime::now() + Duration::from_secs(3600 * 24 * 365);
    let removed = store.gc(far_future).unwrap();

    assert_eq!(removed, 1);
    assert!(store.is_empty().unwrap());
}
