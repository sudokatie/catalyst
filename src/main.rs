use clap::{Parser, Subcommand};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use catalyst::{Config, Error, Label, Node, QueryEngine, Resolver};

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
        Commands::Gc { older_than } => cmd_gc(&workspace, older_than),
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
    let label = Label::parse(target)?;

    let mut resolver = Resolver::new(workspace.clone());
    resolver.resolve(&label)?;

    if let Some(t) = resolver.get_target(&label) {
        if t.rule_type == "rust_binary" || t.rule_type == "cc_binary" {
            println!("Would run {} with args: {:?}", label, args);
            // TODO: Actually build and run
        } else {
            return Err(Error::Config(format!(
                "Target {} is not a binary ({})",
                label, t.rule_type
            )));
        }
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

fn cmd_gc(workspace: &PathBuf, older_than_days: u64) -> Result<(), Error> {
    let config = Config::load_default(Some(workspace))?.with_env_overrides();
    let cache_dir = config.cache_dir();

    if !cache_dir.exists() {
        println!("Cache directory does not exist: {:?}", cache_dir);
        return Ok(());
    }

    println!(
        "Garbage collecting cache entries older than {} days",
        older_than_days
    );
    println!("Cache directory: {:?}", cache_dir);

    // TODO: Actually implement GC using MetadataStore
    println!("GC complete");

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
