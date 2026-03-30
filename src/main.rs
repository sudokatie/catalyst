use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "catalyst")]
#[command(about = "Build system with hermetic builds and content-addressed caching")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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
        /// Query mode: deps, rdeps, graph
        mode: String,

        /// Target to query
        target: String,

        /// Output format (text, dot)
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Remove build outputs
    Clean,

    /// Garbage collect the cache
    Gc {
        /// Maximum cache size in MB
        #[arg(long)]
        max_size: Option<u64>,
    },

    /// Show build configuration
    Info,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    tracing_subscriber::fmt::init();

    let result = match cli.command {
        Commands::Build { targets, jobs } => cmd_build(&targets, jobs),
        Commands::Test { targets } => cmd_test(&targets),
        Commands::Run { target, args } => cmd_run(&target, &args),
        Commands::Query { mode, target, output } => cmd_query(&mode, &target, &output),
        Commands::Clean => cmd_clean(),
        Commands::Gc { max_size } => cmd_gc(max_size),
        Commands::Info => cmd_info(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(targets: &[String], _jobs: Option<usize>) -> Result<(), catalyst::Error> {
    println!("Building targets: {}", targets.join(", "));
    // TODO: implement
    Ok(())
}

fn cmd_test(targets: &[String]) -> Result<(), catalyst::Error> {
    println!("Testing targets: {}", targets.join(", "));
    // TODO: implement
    Ok(())
}

fn cmd_run(target: &str, args: &[String]) -> Result<(), catalyst::Error> {
    println!("Running {target} with args: {}", args.join(" "));
    // TODO: implement
    Ok(())
}

fn cmd_query(mode: &str, target: &str, output: &str) -> Result<(), catalyst::Error> {
    println!("Query {mode} for {target} (format: {output})");
    // TODO: implement
    Ok(())
}

fn cmd_clean() -> Result<(), catalyst::Error> {
    println!("Cleaning build outputs");
    // TODO: implement
    Ok(())
}

fn cmd_gc(max_size: Option<u64>) -> Result<(), catalyst::Error> {
    println!("Garbage collecting cache (max: {max_size:?} MB)");
    // TODO: implement
    Ok(())
}

fn cmd_info() -> Result<(), catalyst::Error> {
    println!("Catalyst Build System v{}", env!("CARGO_PKG_VERSION"));
    // TODO: show config
    Ok(())
}
