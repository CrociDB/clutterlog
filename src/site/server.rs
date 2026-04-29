use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use actix_files::Files;
use actix_web::{App, HttpServer};

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

    if watch {
        println!("Watching for changes...");
        let sp = site_path.clone();
        let url = base_url_override.clone();
        thread::spawn(move || {
            watch_loop(sp, url);
        });
    }

    let build_dir_server = build_dir.clone();
    let server = HttpServer::new(move || {
        App::new().service(Files::new("/", build_dir_server.clone()).index_file("index.html"))
    })
    .bind(&addr)?
    .run();

    actix_web::rt::System::new().block_on(server)
}

fn watch_loop(site_path: PathBuf, base_url_override: Option<String>) {
    let media_path = site_path.join("media");
    let site_toml = site_path.join("site.toml");

    let mut last_site_toml_mtime = mtime(&site_toml);
    let mut media_mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();

    sync_mtimes(&media_path, &mut media_mtimes);

    loop {
        thread::sleep(Duration::from_millis(500));

        let mut changed = false;

        if let Some(current) = mtime(&site_toml)
            && last_site_toml_mtime != Some(current)
        {
            changed = true;
            last_site_toml_mtime = Some(current);
        }

        let mut current: HashMap<PathBuf, SystemTime> = HashMap::new();
        sync_mtimes(&media_path, &mut current);

        for (path, mtime) in &current {
            match media_mtimes.get(path) {
                Some(old) if old != mtime => {
                    changed = true;
                    break;
                }
                None => {
                    changed = true;
                    break;
                }
                _ => {}
            }
        }

        if !changed {
            for path in media_mtimes.keys() {
                if !current.contains_key(path) {
                    changed = true;
                    break;
                }
            }
        }

        media_mtimes = current;

        if changed {
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

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn sync_mtimes(dir: &Path, map: &mut HashMap<PathBuf, SystemTime>) {
    map.clear();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(t) = mtime(&path)
            {
                map.insert(path, t);
            }
        }
    }
}
