use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use actix_files::Files;
use actix_web::{App, HttpServer};
use notify::{RecursiveMode, Watcher};

use super::website::Website;

pub fn serve(
    build_dir: PathBuf,
    port: u16,
    watch: bool,
    site_path: PathBuf,
    base_url_override: Option<String>,
) -> std::io::Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    println!("Serving at http://{}", addr);
    println!("Build directory: {}", build_dir.display());

    let shutdown = Arc::new(AtomicBool::new(false));

    if watch {
        println!("Watching for changes...");
        let sp = site_path.clone();
        let url = base_url_override.clone();
        let shutdown_w = shutdown.clone();
        thread::spawn(move || {
            watch_loop(sp, url, shutdown_w);
        });
    }

    let shutdown_s = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_s.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let build_dir_server = build_dir.clone();
    let server = HttpServer::new(move || {
        App::new().service(Files::new("/", build_dir_server.clone()).index_file("index.html"))
    })
    .bind(&addr)?
    .run();

    actix_web::rt::System::new().block_on(server)
}

fn watch_loop(site_path: PathBuf, base_url_override: Option<String>, shutdown: Arc<AtomicBool>) {
    let (tx, rx) = channel();

    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to create file watcher: {}", e);
            return;
        }
    };

    let media_path = site_path.join("media");
    let site_toml = site_path.join("site.toml");

    if media_path.exists()
        && let Err(e) = watcher.watch(&media_path, RecursiveMode::Recursive)
    {
        eprintln!("Failed to watch media directory: {}", e);
    }

    if site_toml.exists()
        && let Err(e) = watcher.watch(&site_toml, RecursiveMode::NonRecursive)
    {
        eprintln!("Failed to watch site.toml: {}", e);
    }

    let debounce_duration = Duration::from_millis(300);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            println!("Shutting down watcher...");
            break;
        }

        if rx.recv_timeout(debounce_duration).is_ok() {
            while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}

            if shutdown.load(Ordering::SeqCst) {
                break;
            }

            println!("Change detected, rebuilding...");
            match Website::load(&site_path) {
                Ok(website) => match website.build(base_url_override.as_deref()) {
                    Ok(report) => println!("Rebuilt successfully\n{}", report),
                    Err(e) => eprintln!("Error rebuilding: {}", e),
                },
                Err(e) => eprintln!("Error loading site: {}", e),
            }
        }
    }
}
