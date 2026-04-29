use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rayon::prelude::*;

use super::media_library::{MediaLibrary, MediaLibraryError};
use super::website_info::{SITE_TOML, WebsiteInfo, WebsiteInfoError};
use super::website_media::{GenerationResult, WebsiteMedia};

const DEFAULT_BUILD_DIR: &str = "build";
const DEFAULT_MEDIA_DIR: &str = "media";
const DEFAULT_PUBLIC_DIR: &str = "public";

const DEFAULT_FEED_FILE: &str = "feed.xml";

const TEMPLATE_INDEX: &str = include_str!("../../template/index.html");
const TEMPLATE_STYLE: &str = include_str!("../../template/public/style.css");
const TEMPLATE_JS: &str = include_str!("../../template/public/clutterlog.js");
const TEMPLATE_GITHUB_ACTION: &str = include_str!("../../template/github_action.yaml");
const TEMPLATE_RSS: &str = include_str!("../../template/rss.xml");

pub struct BuildReport {
    pub items_processed: usize,
    pub items_skipped: usize,
    pub total_media_size: u64,
    pub total_thumbs_size: u64,
    pub processing_time: Duration,
}

impl BuildReport {
    fn from_results(
        results: Vec<GenerationResult>,
        items_skipped: usize,
        processing_time: Duration,
    ) -> Self {
        let items_processed = results.len();
        let total_media_size = results.iter().map(|r| r.media_size).sum();
        let total_thumbs_size = results.iter().map(|r| r.thumb_size).sum();
        Self {
            items_processed,
            items_skipped,
            total_media_size,
            total_thumbs_size,
            processing_time,
        }
    }
}

impl std::fmt::Display for BuildReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Build report:")?;
        writeln!(f, "  Items processed: {}", self.items_processed)?;
        writeln!(f, "  Items skipped (up to date): {}", self.items_skipped)?;
        writeln!(
            f,
            "  Total media size: {}",
            format_size(self.total_media_size)
        )?;
        writeln!(
            f,
            "  Total thumbs size: {}",
            format_size(self.total_thumbs_size)
        )?;
        write!(
            f,
            "  Processing time: {}",
            format_duration(self.processing_time)
        )
    }
}

fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs_f64();
    if total_secs < 1.0 {
        format!("{:.0}ms", duration.as_millis())
    } else if total_secs < 60.0 {
        format!("{:.2}s", total_secs)
    } else {
        let mins = duration.as_secs() / 60;
        let secs = total_secs - (mins as f64 * 60.0);
        format!("{}m {:.2}s", mins, secs)
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[derive(Debug)]
pub struct Website {
    pub info: WebsiteInfo,
    pub path: PathBuf,
}

impl Website {
    pub fn load(path: &Path) -> Result<Self, WebsiteError> {
        if !path.join(SITE_TOML).exists() {
            let abs = path
                .canonicalize()
                .map_err(|_| WebsiteError::NotAPath(path.to_path_buf()))?;
            return Err(WebsiteError::NotASite(abs.to_path_buf()));
        }

        let info = WebsiteInfo::from_file(path)?;
        Ok(Self {
            info,
            path: path.to_path_buf(),
        })
    }

    pub fn new(path: &Path) -> Result<Self, WebsiteError> {
        let website = if path.join(SITE_TOML).exists() {
            let info = WebsiteInfo::from_file(path)?;
            Self {
                info,
                path: path.to_path_buf(),
            }
        } else {
            fs::create_dir_all(path).map_err(|e| WebsiteError::Io(path.to_path_buf(), e))?;

            let media_path = path.join(DEFAULT_MEDIA_DIR);
            fs::create_dir_all(&media_path)
                .map_err(|e| WebsiteError::Io(media_path.to_path_buf(), e))?;

            let title = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("untitled");

            let info = WebsiteInfo::new(title);
            let toml_content = toml::to_string_pretty(&info).map_err(WebsiteError::Serialize)?;
            let file_path = path.join(SITE_TOML);
            fs::write(&file_path, &toml_content).map_err(|e| WebsiteError::Io(file_path, e))?;

            Self {
                info,
                path: path.to_path_buf(),
            }
        };

        // Always write (or overwrite) the GitHub Actions deploy workflow
        let workflows_path = path.join(".github").join("workflows");
        fs::create_dir_all(&workflows_path)
            .map_err(|e| WebsiteError::Io(workflows_path.clone(), e))?;
        let deploy_path = workflows_path.join("deploy.yml");
        fs::write(&deploy_path, TEMPLATE_GITHUB_ACTION)
            .map_err(|e| WebsiteError::Io(deploy_path, e))?;

        Ok(website)
    }

    pub fn build(&self, base_url_override: Option<&str>) -> Result<BuildReport, WebsiteError> {
        let start = Instant::now();
        let base_url = base_url_override
            .unwrap_or(&self.info.url)
            .trim_end_matches('/');
        let build_path = self.path.join(DEFAULT_BUILD_DIR);
        fs::create_dir_all(&build_path).map_err(|e| WebsiteError::Io(build_path.clone(), e))?;

        let build_media_path = build_path.join(DEFAULT_MEDIA_DIR);
        fs::create_dir_all(&build_media_path)
            .map_err(|e| WebsiteError::Io(build_media_path.clone(), e))?;

        let public_path = build_path.join(DEFAULT_PUBLIC_DIR);
        fs::create_dir_all(&public_path).map_err(|e| WebsiteError::Io(public_path.clone(), e))?;

        // Update media metadata before scanning
        let source_media_path = self.path.join(DEFAULT_MEDIA_DIR);
        let mut library = MediaLibrary::new(&self.path)?;
        library.update_metadata(&source_media_path)?;

        // Scan source media directory, copy files, generate thumbnails, and collect data entries
        let (clutterlog_data, rss_items, generation_results, items_skipped) =
            self.scan_and_copy_media(&source_media_path, &build_media_path, &library, base_url)?;

        // Render index.html from template
        let rendered = TEMPLATE_INDEX
            .replace("{{title}}", &escape_html(&self.info.title))
            .replace("{{description}}", &escape_html(&self.info.description))
            .replace("{{author}}", &escape_html(&self.info.author))
            .replace("{{clutterlog_data}}", &clutterlog_data);

        let index_path = build_path.join("index.html");
        fs::write(&index_path, &rendered).map_err(|e| WebsiteError::Io(index_path, e))?;

        // Render and write feed.xml
        let rss_items_str = if rss_items.is_empty() {
            String::new()
        } else {
            format!("{}\n", rss_items.join("\n"))
        };
        let feed_url = format!("{}/", base_url);
        let rss_rendered = TEMPLATE_RSS
            .replace("{{title}}", &escape_html(&self.info.title))
            .replace("{{url}}", &escape_html(&feed_url))
            .replace("{{description}}", &escape_html(&self.info.description))
            .replace("{{items}}", &rss_items_str);

        let rss_path = build_path.join(DEFAULT_FEED_FILE);
        fs::write(&rss_path, &rss_rendered).map_err(|e| WebsiteError::Io(rss_path, e))?;

        // Write static assets
        let style_path = public_path.join("style.css");
        fs::write(&style_path, TEMPLATE_STYLE).map_err(|e| WebsiteError::Io(style_path, e))?;

        let js_path = public_path.join("clutterlog.js");
        fs::write(&js_path, TEMPLATE_JS).map_err(|e| WebsiteError::Io(js_path, e))?;

        Ok(BuildReport::from_results(
            generation_results,
            items_skipped,
            start.elapsed(),
        ))
    }

    fn scan_and_copy_media(
        &self,
        source_path: &Path,
        dest_path: &Path,
        library: &MediaLibrary,
        base_url: &str,
    ) -> Result<(String, Vec<String>, Vec<GenerationResult>, usize), WebsiteError> {
        if !source_path.exists() {
            return Ok(("[]".to_string(), Vec::new(), Vec::new(), 0));
        }

        let dir_entries = fs::read_dir(source_path)
            .map_err(|e| WebsiteError::Io(source_path.to_path_buf(), e))?;

        // Collect directory entries so we can process them in parallel
        let items: Vec<(PathBuf, Option<String>)> = dir_entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str())?;
                let datetime = library.get_datetime(filename).map(|s| s.to_string());
                Some((path, datetime))
            })
            .collect();

        // Process items in parallel: copy files and generate thumbnails (skipping up-to-date items)
        // Each result includes a bool indicating whether the item was skipped.
        let processed: Vec<Result<(GenerationResult, String, String, bool), WebsiteError>> = items
            .par_iter()
            .filter_map(|(path, datetime)| {
                let item = WebsiteMedia::from_path(path, datetime.as_deref())?;

                let (result, skipped) = if item.is_up_to_date(dest_path) {
                    (item.read_existing_sizes(dest_path), true)
                } else {
                    (item.copy_and_generate_thumb(dest_path), false)
                };

                let entry = item.to_json_entry(base_url, DEFAULT_MEDIA_DIR);
                let rss_item = item.to_rss_item(base_url, DEFAULT_MEDIA_DIR);
                let image_url = item.image_url(base_url, DEFAULT_MEDIA_DIR);
                Some(result.map(|mut r| {
                    r.image_url = image_url;
                    (r, entry, rss_item, skipped)
                }))
            })
            .collect();

        // Collect results, propagating any errors
        let mut results: Vec<GenerationResult> = Vec::new();
        let mut entries: Vec<String> = Vec::new();
        let mut rss_items: Vec<String> = Vec::new();
        let mut items_skipped: usize = 0;
        for item_result in processed {
            let (gen_result, entry, rss_item, skipped) = item_result?;
            if skipped {
                items_skipped += 1;
            }
            results.push(gen_result);
            entries.push(entry);
            rss_items.push(rss_item);
        }

        let json = if entries.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n        ]", entries.join(",\n"))
        };

        Ok((json, rss_items, results, items_skipped))
    }
}

// Error

#[derive(Debug)]
pub enum WebsiteError {
    NotASite(PathBuf),
    NotAPath(PathBuf),
    Info(Box<WebsiteInfoError>),
    MediaLibrary(Box<MediaLibraryError>),
    Io(PathBuf, io::Error),
    Serialize(toml::ser::Error),
    Image(PathBuf, image::ImageError),
    Ffmpeg(PathBuf, String),
    FfmpegNotFound(String),
}

impl std::fmt::Display for WebsiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebsiteError::NotASite(path) => {
                write!(
                    f,
                    "'{}' is not a clutterlog site (missing site.toml)",
                    path.display()
                )
            }
            WebsiteError::NotAPath(path) => {
                let abs = if path.is_absolute() {
                    path.to_path_buf()
                } else if let Ok(currentdir) = std::env::current_dir() {
                    currentdir.join(path)
                } else {
                    path.to_path_buf()
                };

                write!(f, "'{}' is not a valid path", abs.display())
            }
            WebsiteError::Info(err) => write!(f, "{}", err),
            WebsiteError::MediaLibrary(err) => write!(f, "{}", err),
            WebsiteError::Io(path, err) => {
                write!(f, "failed to write '{}': {}", path.display(), err)
            }
            WebsiteError::Serialize(err) => write!(f, "failed to serialize site info: {}", err),
            WebsiteError::Image(path, err) => {
                write!(f, "failed to process image '{}': {}", path.display(), err)
            }
            WebsiteError::Ffmpeg(path, err) => {
                write!(
                    f,
                    "failed to extract video frame from '{}': {}",
                    path.display(),
                    err
                )
            }
            WebsiteError::FfmpegNotFound(err) => {
                write!(f, "looks like `ffmpeg` is not installed: {}", err)
            }
        }
    }
}

impl std::error::Error for WebsiteError {}

impl From<WebsiteInfoError> for WebsiteError {
    fn from(err: WebsiteInfoError) -> Self {
        WebsiteError::Info(Box::new(err))
    }
}

impl From<MediaLibraryError> for WebsiteError {
    fn from(err: MediaLibraryError) -> Self {
        WebsiteError::MediaLibrary(Box::new(err))
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
