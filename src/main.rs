mod site;

use std::path::Path;

use clap::{Parser, Subcommand};
use site::{MediaLibrary, Website};

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new clutterlog site
    New {
        /// Name of the site to create
        site_name: String,
    },
    /// Build the site in the current directory
    Build,
    /// Build the site and serve it via HTTP
    Server {
        /// Override the site URL
        #[arg(short = 'b')]
        url: Option<String>,
        /// Watch for file changes and rebuild
        #[arg(long)]
        watch: bool,
        /// Port to listen on
        #[arg(short = 'p', long, default_value = "8088")]
        port: u16,
    },
    /// Update media metadata in the current directory
    Update,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { site_name } => {
            let path = Path::new(&site_name);
            match Website::new(path) {
                Ok(website) => {
                    println!(
                        "Created new site '{}' at '{}'",
                        website.info.title,
                        website.path.display()
                    );
                }
                Err(e) => {
                    eprintln!("Error creating site: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Build => {
            let path = Path::new(".");
            match Website::load(path) {
                Ok(website) => match website.build(None) {
                    Ok(report) => {
                        println!("Site '{}' built successfully\n", website.info.title);
                        println!("{}", report);
                    }
                    Err(e) => {
                        eprintln!("Error building site: {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Server { url, watch, port } => {
            let path = Path::new(".");
            match Website::load(path) {
                Ok(website) => {
                    match website.build(url.as_deref()) {
                        Ok(report) => {
                            println!(
                                "Site '{}' built successfully\n{}",
                                website.info.title, report
                            );
                        }
                        Err(e) => {
                            eprintln!("Error building site: {}", e);
                            std::process::exit(1);
                        }
                    }

                    let build_dir = website.path.join("build");
                    if let Err(e) =
                        site::server::serve(build_dir, port, watch, website.path.clone(), url)
                    {
                        eprintln!("Server error: {}", e);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Update => {
            let path = Path::new(".");
            match Website::load(path) {
                Ok(website) => {
                    let mut library = match MediaLibrary::new(&website.path) {
                        Ok(lib) => lib,
                        Err(e) => {
                            eprintln!("Error loading media library: {}", e);
                            std::process::exit(1);
                        }
                    };

                    let media_path = website.path.join("media");
                    match library.update_metadata(&media_path) {
                        Ok(report) => {
                            println!("Updated metadata: {}", report);
                        }
                        Err(e) => {
                            eprintln!("Error updating metadata: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
