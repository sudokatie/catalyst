use clap::{Parser, Subcommand};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use catalyst::{
    expand_target, Config, Error, Label, MetadataStore, Node, QueryEngine, Resolver,
    ScheduledAction, Scheduler,
};

#[derive(Parser)]
#[command(name = "catalyst")]
#[command(about = "Build system with hermetic builds and content-addressed caching")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Workspace root directory
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Build specified targets
    Build {
        /// Targets to build (e.g., //pkg:target)
        #[arg(required = true)]
        targets: Vec<String>,

        /// Number of parallel jobs
        #[arg(short, long)]
        jobs: Option<usize>,

        /// Keep going on failure
        #[arg(short, long)]
        keep_going: bool,
    },

    /// Build and run tests
    Test {
        /// Targets to test
        #[arg(required = true)]
        targets: Vec<String>,
    },

    /// Build and run a binary
    Run {
        /// Target to run
        target: String,

        /// Arguments to pass to the binary
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Query the dependency graph
    Query {
        /// Query mode: deps, rdeps
        mode: String,

        /// Target to query
        target: String,

        /// Output format (text, dot)
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Remove build outputs
    Clean {
        /// Also remove the cache
        #[arg(long)]
        expunge: bool,
    },

    /// Garbage collect the cache
    Gc {
        /// Remove entries older than N days
        #[arg(long, default_value = "30")]
        older_than: u64,

        /// Show what would be deleted without actually deleting
        #[arg(long)]
        dry_run: bool,

        /// List each deleted file
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show build configuration
    Info,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    tracing_subscriber::fmt::init();

    let workspace = cli.workspace.unwrap_or_else(find_workspace_root);

    let result = match cli.command {
        Commands::Build {
            targets,
            jobs,
            keep_going,
        } => cmd_build(&workspace, &targets, jobs, keep_going),
        Commands::Test { targets } => cmd_test(&workspace, &targets),
        Commands::Run { target, args } => cmd_run(&workspace, &target, &args),
        Commands::Query {
            mode,
            target,
            output,
        } => cmd_query(&workspace, &mode, &target, &output),
        Commands::Clean { expunge } => cmd_clean(&workspace, expunge),
        Commands::Gc {
            older_than,
            dry_run,
            verbose,
        } => cmd_gc(&workspace, older_than, dry_run, verbose),
        Commands::Info => cmd_info(&workspace),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Find the workspace root by looking for WORKSPACE file
fn find_workspace_root() -> PathBuf {
    let mut current = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if current.join("WORKSPACE").exists() {
            return current;
        }
        if !current.pop() {
            // No WORKSPACE found, use current directory
            return env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
}

fn cmd_build(
    workspace: &PathBuf,
    targets: &[String],
    jobs: Option<usize>,
    _keep_going: bool,
) -> Result<(), Error> {
    let config = Config::load_default(Some(workspace))?.with_env_overrides();
    let jobs = jobs.unwrap_or_else(|| config.jobs());

    println!("Building {} target(s) with {} jobs", targets.len(), jobs);

    let mut resolver = Resolver::new(workspace.clone());

    for target_str in targets {
        let label = Label::parse(target_str)?;
        println!("  Resolving {}", label);
        resolver.resolve(&label)?;
    }

    let graph = resolver.graph();
    let order = graph.topo_order()?;

    println!("Build order ({} targets):", order.len());
    for node_id in &order {
        if let Some(Node::Target(t)) = graph.get(*node_id) {
            println!("  - {}", t.label);
        }
    }

    println!("Build complete.");
    Ok(())
}

fn cmd_test(workspace: &PathBuf, targets: &[String]) -> Result<(), Error> {
    println!("Testing {} target(s):", targets.len());

    let mut resolver = Resolver::new(workspace.clone());

    for target_str in targets {
        let label = Label::parse(target_str)?;
        resolver.resolve(&label)?;

        if let Some(target) = resolver.get_target(&label) {
            if target.rule_type.contains("test") {
                println!("  Running test: {}", label);
            } else {
                println!("  Skipping non-test target: {}", label);
            }
        }
    }

    Ok(())
}

fn cmd_run(workspace: &PathBuf, target: &str, args: &[String]) -> Result<(), Error> {
    let config = Config::load_default(Some(workspace))?.with_env_overrides();
    let label = Label::parse(target)?;

    let mut resolver = Resolver::new(workspace.clone());
    resolver.resolve(&label)?;

    let t = resolver
        .get_target(&label)
        .ok_or_else(|| Error::UnknownTarget(label.to_string()))?
        .clone();

    if t.rule_type != "rust_binary" && t.rule_type != "cc_binary" {
        return Err(Error::Config(format!(
            "Target {} is not a binary ({})",
            label, t.rule_type
        )));
    }

    // Build the target first
    let output_dir = workspace.join(".catalyst").join("out");
    std::fs::create_dir_all(&output_dir)?;

    // Expand target into actions
    let expansion = expand_target(&t, &output_dir);

    if expansion.actions.is_empty() {
        return Err(Error::Config(format!(
            "Target {} has no build actions",
            label
        )));
    }

    println!("Building {}...", label);

    // Build dependencies first, then the target
    let graph = resolver.graph();

    // Get topological order for building
    let topo = graph.topo_order()?;

    // Collect all actions from dependencies and target
    let rt = tokio::runtime::Runtime::new().map_err(|e| Error::Io(e.into()))?;

    let build_result = rt.block_on(async {
        let jobs = config.jobs();
        let (mut scheduler, runner) = Scheduler::new(jobs);
        tokio::spawn(runner.run());

        // Build all dependencies in order, then the target
        for node_id in &topo {
            if let Some(Node::Target(target_node)) = graph.get(*node_id) {
                if let Some(dep_target) = resolver.get_target(&target_node.label) {
                    let dep_expansion = expand_target(dep_target, &output_dir);
                    for action in dep_expansion.actions {
                        let mut scheduled = ScheduledAction::new(action.clone());
                        scheduled.action.set_working_dir(workspace.clone());
                        scheduler.add(scheduled);
                    }
                }
            }
        }

        scheduler.execute().await
    })?;

    if !build_result.success() {
        let failed_count = build_result.failed;
        return Err(Error::ActionFailed {
            command: format!("build {}", label),
            exit_code: 1,
            stderr: format!("{} action(s) failed", failed_count),
        });
    }

    println!("Build succeeded ({} actions)", build_result.succeeded);

    // Find the output binary
    let binary_path = if expansion.outputs.is_empty() {
        output_dir.join(&label.name)
    } else {
        expansion.outputs[0].clone()
    };

    if !binary_path.exists() {
        return Err(Error::Config(format!(
            "Binary not found at {:?}",
            binary_path
        )));
    }

    // Run the binary
    println!("Running {}...", binary_path.display());
    let status = std::process::Command::new(&binary_path)
        .args(args)
        .current_dir(workspace)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| Error::Io(e))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        return Err(Error::ActionFailed {
            command: binary_path.to_string_lossy().to_string(),
            exit_code: code,
            stderr: format!("Process exited with code {}", code),
        });
    }

    Ok(())
}

fn cmd_query(workspace: &PathBuf, mode: &str, target: &str, output: &str) -> Result<(), Error> {
    let label = Label::parse(target)?;

    let mut resolver = Resolver::new(workspace.clone());
    let node_id = resolver.resolve(&label)?;

    let query = QueryEngine::new(resolver.graph());

    match mode {
        "deps" => {
            let deps = query.transitive_deps(node_id);
            if output == "dot" {
                print!("{}", query.subgraph_to_dot(node_id));
            } else {
                println!("Dependencies of {}:", label);
                for dep_id in deps {
                    if let Some(Node::Target(t)) = query.get(dep_id) {
                        println!("  {}", t.label);
                    }
                }
            }
        }
        "rdeps" => {
            let rdeps = query.transitive_rdeps(node_id);
            println!("Reverse dependencies of {}:", label);
            for rdep_id in rdeps {
                if let Some(Node::Target(t)) = query.get(rdep_id) {
                    println!("  {}", t.label);
                }
            }
        }
        _ => {
            return Err(Error::Config(format!("Unknown query mode: {}", mode)));
        }
    }

    Ok(())
}

fn cmd_clean(workspace: &PathBuf, expunge: bool) -> Result<(), Error> {
    let output_dir = workspace.join(".catalyst");

    if output_dir.exists() {
        if expunge {
            println!("Removing all build outputs and cache: {:?}", output_dir);
            std::fs::remove_dir_all(&output_dir)?;
        } else {
            let outputs = output_dir.join("out");
            if outputs.exists() {
                println!("Removing build outputs: {:?}", outputs);
                std::fs::remove_dir_all(&outputs)?;
            } else {
                println!("No build outputs to clean");
            }
        }
    } else {
        println!("Nothing to clean");
    }

    Ok(())
}

fn cmd_gc(
    workspace: &PathBuf,
    older_than_days: u64,
    dry_run: bool,
    verbose: bool,
) -> Result<(), Error> {
    let config = Config::load_default(Some(workspace))?.with_env_overrides();
    let cache_dir = config.cache_dir();

    if !cache_dir.exists() {
        println!("Cache directory does not exist: {:?}", cache_dir);
        return Ok(());
    }

    let mode = if dry_run { " (dry-run)" } else { "" };
    println!(
        "Garbage collecting cache entries older than {} days{}",
        older_than_days, mode
    );
    println!("Cache directory: {:?}", cache_dir);

    // Calculate cutoff time
    let cutoff = SystemTime::now() - Duration::from_secs(older_than_days * 24 * 60 * 60);

    // Initialize metadata store path
    let metadata_path = cache_dir.join("metadata.db");

    let mut files_deleted = 0usize;
    let mut bytes_freed = 0u64;
    let mut files_kept = 0usize;

    // GC from MetadataStore if it exists
    if metadata_path.exists() {
        let store = MetadataStore::new(&metadata_path)?;

        if dry_run {
            // In dry-run mode, just count what would be deleted
            let all_paths = store.all_paths()?;
            for path in &all_paths {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if let Ok(accessed) = metadata.accessed() {
                        if accessed < cutoff {
                            files_deleted += 1;
                            bytes_freed += metadata.len();
                            if verbose {
                                println!("  Would delete: {:?}", path);
                            }
                        } else {
                            files_kept += 1;
                        }
                    }
                }
            }
        } else {
            let removed = store.gc(cutoff)?;
            files_deleted += removed;
            if verbose && removed > 0 {
                println!("  Removed {} metadata entries", removed);
            }
        }
    }

    // GC from action cache directory
    let action_cache_dir = cache_dir.join("actions");
    if action_cache_dir.exists() {
        gc_directory(&action_cache_dir, cutoff, dry_run, verbose, &mut files_deleted, &mut bytes_freed, &mut files_kept)?;
    }

    // GC from CAS directory
    let cas_dir = cache_dir.join("cas");
    if cas_dir.exists() {
        gc_directory(&cas_dir, cutoff, dry_run, verbose, &mut files_deleted, &mut bytes_freed, &mut files_kept)?;
    }

    println!();
    if dry_run {
        println!("Would delete {} files ({} bytes)", files_deleted, bytes_freed);
        println!("Would keep {} files", files_kept);
    } else {
        println!("Deleted {} files ({} bytes freed)", files_deleted, bytes_freed);
        println!("Kept {} files", files_kept);
    }
    println!("GC complete");

    Ok(())
}

/// Garbage collect files in a directory that haven't been accessed since cutoff
fn gc_directory(
    dir: &PathBuf,
    cutoff: SystemTime,
    dry_run: bool,
    verbose: bool,
    files_deleted: &mut usize,
    bytes_freed: &mut u64,
    files_kept: &mut usize,
) -> Result<(), Error> {
    gc_directory_recursive(dir, cutoff, dry_run, verbose, files_deleted, bytes_freed, files_kept)
}

fn gc_directory_recursive(
    dir: &PathBuf,
    cutoff: SystemTime,
    dry_run: bool,
    verbose: bool,
    files_deleted: &mut usize,
    bytes_freed: &mut u64,
    files_kept: &mut usize,
) -> Result<(), Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            gc_directory_recursive(&path, cutoff, dry_run, verbose, files_deleted, bytes_freed, files_kept)?;
        } else if file_type.is_file() {
            if let Ok(metadata) = std::fs::metadata(&path) {
                // Use mtime since atime may not be reliable
                let file_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if file_time < cutoff {
                    *files_deleted += 1;
                    *bytes_freed += metadata.len();
                    if verbose {
                        println!(
                            "  {}: {:?}",
                            if dry_run { "Would delete" } else { "Deleting" },
                            path
                        );
                    }
                    if !dry_run {
                        let _ = std::fs::remove_file(&path);
                    }
                } else {
                    *files_kept += 1;
                }
            }
        }
    }
    Ok(())
}

fn cmd_info(workspace: &PathBuf) -> Result<(), Error> {
    let config = Config::load_default(Some(workspace))?.with_env_overrides();

    println!("Catalyst Build System v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Workspace: {:?}", workspace);
    println!();
    println!("Build Configuration:");
    println!("  Jobs: {}", config.jobs());
    println!("  Sandbox: {}", config.build.sandbox);
    println!("  Verbose: {}", config.build.verbose);
    println!();
    println!("Cache Configuration:");
    println!("  Local: {:?}", config.cache_dir());
    println!(
        "  Remote: {}",
        config.cache.remote.as_deref().unwrap_or("none")
    );
    println!();
    println!("Remote Execution:");
    println!(
        "  Executor: {}",
        config.remote.executor.as_deref().unwrap_or("none")
    );

    Ok(())
}
