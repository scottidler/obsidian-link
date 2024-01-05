#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use log::{debug, info, warn, error};
use std::path::{Path, PathBuf};
use std::io::Write;
use std::env;
use std::collections::HashMap;

use reqwest;
use tokio;
use shellexpand;
use chrono::prelude::*;
use chrono_tz::Tz;
use chrono::format::StrftimeItems;
use regex::Regex;
use clap::{Parser, Args};
use serde::Deserialize;
use eyre::{eyre, Result};
use lazy_static::lazy_static;

const TIMEZONE: &str = "America/Los_Angeles";

lazy_static! {
    static ref LOG_LEVEL: String = std::env::var("LOG_LEVEL").unwrap_or("INFO".to_string());
    static ref YOUTUBE_API_KEY: String = env::var("YOUTUBE_API_KEY").expect("YOUTUBE_API_KEY not set in environment");
    static ref CHATGPT_API_KEY: String = env::var("CHATGPT_API_KEY").expect("CHATGPT_API_KEY not set in environment");
    static ref RESOLUTIONS: HashMap<&'static str, (usize, usize)> = {
        let mut m = HashMap::new();
        m.insert("nHD", (640, 360));
        m.insert("FWVGA", (854, 480));
        m.insert("qHD", (960, 540));
        m.insert("SD", (1280, 720));
        m.insert("WXGA", (1366, 768));
        m.insert("HD+", (1600, 900));
        m.insert("FHD", (1920, 1080));
        m.insert("WQHD", (2560, 1440));
        m.insert("QHD+", (3200, 1800));
        m.insert("4K", (3840, 2160));
        m.insert("5K", (5120, 2880));
        m.insert("8K", (7680, 4320));
        m.insert("16K", (15360, 8640));
        m
    };
    static ref SHORTS_RESOLUTIONS: HashMap<&'static str, (usize, usize)> = {
        let mut m = HashMap::new();
        m.insert("480p", (480, 854));
        m.insert("720p", (720, 1280));
        m.insert("1080p", (1080, 1920));
        m.insert("1440p", (1440, 2560));
        m.insert("2160p", (2160, 3840));
        m
    };
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, value_parser, default_value = "~/.config/obsidian-link/obsidian-link.yml")]
    config: PathBuf,

    #[clap(short, long)]
    url: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Config {
    vault: PathBuf,
    frontmatter: Frontmatter,
    links: Vec<Link>,
}

#[derive(Deserialize, Debug)]
struct Frontmatter {
    date: Option<String>,
    day: Option<String>,
    time: Option<String>,
    tags: Option<Vec<String>>,
    url: Option<String>,
    author: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Link {
    name: String,
    regex: String,
    resolution: String,
    folder: String,
}

#[derive(Debug)]
struct VideoMetadata {
    id: String,
    title: String,
    description: String,
    channel: String,
    published_at: String,
    tags: Vec<String>,
}

enum LinkType {
    Shorts(String, String, usize, usize),
    YouTube(String, String, usize, usize),
    WebLink(String, String, usize, usize),
}

impl LinkType {
    fn from_url(url: &str, config: &Config) -> Result<LinkType> {
        debug!("LinkType::from_url: url={} config={:?}", url, config);
        let mut default_link = None;

        for link in &config.links {
            let regex = Regex::new(&link.regex)?;
            if regex.is_match(url) {
                if link.name == "default" {
                    default_link = Some(LinkType::WebLink(url.to_string(), link.folder.clone(), 0, 0));
                    continue;
                }
                let (width, height) = get_resolution(&link.name, config)?;
                return Ok(match link.name.as_str() {
                    "shorts" => LinkType::Shorts(url.to_string(), link.folder.clone(), width, height),
                    "youtube" => LinkType::YouTube(url.to_string(), link.folder.clone(), width, height),
                    _ => LinkType::WebLink(url.to_string(), link.folder.clone(), width, height),
                });
            }
        }

        if let Some(default_link) = default_link {
            return Ok(default_link);
        }

        Err(eyre!("Invalid URL format"))
    }
}

fn expanduser<T: AsRef<str>>(path: T) -> Result<PathBuf> {
    let expanded_path_str = shellexpand::tilde(path.as_ref());
    Ok(PathBuf::from(expanded_path_str.into_owned()))
}

fn today() -> (String, String, String) {
    debug!("today");
    let tz: Tz = TIMEZONE.parse().expect("Invalid timezone");
    let now = Utc::now().with_timezone(&tz);

    let date_format = StrftimeItems::new("%Y-%m-%d");
    let day_format = StrftimeItems::new("%a");
    let time_format = StrftimeItems::new("%H:%M");

    let formatted_date = now.format_with_items(date_format).to_string();
    let formatted_day = now.format_with_items(day_format).to_string();
    let formatted_time = now.format_with_items(time_format).to_string();

    (formatted_date, formatted_day, formatted_time)
}

fn get_resolution(link_name: &str, config: &Config) -> Result<(usize, usize)> {
    debug!("get_resolution: link_name={} config={:?}", link_name, config);
    let resolution_key = config.links.iter().find(|link| link.name == link_name)
        .ok_or_else(|| eyre!("Link type '{}' not found in config", link_name))?.resolution.as_str(); // Convert to &str

    match link_name {
        "shorts" => SHORTS_RESOLUTIONS.get(resolution_key)
            .copied()
            .ok_or_else(|| eyre!("Resolution not found for shorts")),
        "youtube" | _ => RESOLUTIONS.get(resolution_key)
            .copied()
            .ok_or_else(|| eyre!("Resolution not found for {}", link_name)),
    }
}

fn load_config(config_path: PathBuf) -> Result<Config> {
    debug!("load_config: config_path={}", config_path.display());
    let config_path_str = config_path.to_str()
        .ok_or_else(|| eyre!("Failed to convert config path to string"))?;
    let config_path_expanded = expanduser(config_path_str)?;
    let config_str = std::fs::read_to_string(config_path_expanded)
        .map_err(|e| eyre!("Failed to read config file: {}", e))?;
    let config: Config = serde_yaml::from_str(&config_str)
        .map_err(|e| eyre!("Failed to parse config file: {}", e))?;
    Ok(config)
}

fn extract_video_id(url: &str) -> Result<String> {
    debug!("extract_video_id: url={}", url);
    let pattern = Regex::new(r#"(youtu\.be/|youtube\.com/(watch\?(.*&)?v=|(embed|v|shorts)/))([^?&">]+)"#)
        .map_err(|e| eyre!("Failed to compile regex: {}", e))?;

    pattern.captures(url)
        .and_then(|caps| caps.get(5))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| eyre!("Failed to extract video ID from URL"))
}

async fn create_markdown_file(title: &str, description: &str, embed_code: &str, url: &str, author: &str, tags: &[String], vault_path: &PathBuf, folder: &str, frontmatter: &Frontmatter) -> Result<()> {
    debug!("create_markdown_file: title={} description={} embed_code={} url={} author={} tags={:?} vault_path={} folder={} frontmatter={:?}", title, description, embed_code, url, author, tags, vault_path.display(), folder, frontmatter);
    let vault_path_str = vault_path.to_str().ok_or_else(|| eyre!("Failed to convert vault path to string"))?;
    let vault_path_expanded = expanduser(vault_path_str)?;
    let full_path = vault_path_expanded.join(folder);

    std::fs::create_dir_all(&full_path).map_err(|e| eyre!("Failed to create directory: {:?} with error {}", full_path, e))?;

    let file_name = sanitize_filename(title);
    let file_path = full_path.join(file_name + ".md");

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| eyre!("Failed to create markdown file: {:?} with error {}", file_path, e))?;

    let frontmatter_str = format_frontmatter(frontmatter, url, author, tags);
    write!(file, "{}\n{}\n\n## Description\n{}", frontmatter_str, embed_code, description)
        .map_err(|e| eyre!("Failed to write to markdown file: {}", e))
}

fn format_frontmatter(frontmatter: &Frontmatter, url: &str, author: &str, tags: &[String]) -> String {
    debug!("format_frontmatter: frontmatter={:?} url={} author={} tags={:?}", frontmatter, url, author, tags);
    let mut frontmatter_str = String::from("---\n");

    let (current_date, current_day, current_time) = today();
    frontmatter_str += &format!("date: {}\n", frontmatter.date.as_ref().unwrap_or(&current_date));
    frontmatter_str += &format!("day: {}\n", frontmatter.day.as_ref().unwrap_or(&current_day));
    frontmatter_str += &format!("time: {}\n", frontmatter.time.as_ref().unwrap_or(&current_time));

    if !tags.is_empty() {
        frontmatter_str += "tags:\n";
        for tag in tags {
            frontmatter_str += &format!("  - {}\n", sanitize_tag(tag));
        }
    }

    frontmatter_str += &format!("url: {}\n", url);
    frontmatter_str += &format!("author: {}\n", author);

    frontmatter_str += "---\n\n";
    frontmatter_str
}

fn generate_embed_code(video_id: &str, width: usize, height: usize) -> String {
    debug!("generate_embed_code: video_id={} width={} height={}", video_id, width, height);
    format!(
        "<iframe width=\"{}\" height=\"{}\" src=\"https://www.youtube.com/embed/{}\" frameborder=\"0\" allowfullscreen></iframe>",
        width, height, video_id
    )
}

fn sanitize_tag(tag: &str) -> String {
    debug!("sanitize_tag: tag={}", tag);
    tag.replace("'", "")
       .chars()
       .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { '-' })
       .collect::<String>()
       .replace(' ', "-")
       .to_lowercase()
}

fn sanitize_filename(title: &str) -> String {
    debug!("sanitize_filename: title={}", title);
    title.chars()
         .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
         .collect::<String>()
}

async fn fetch_video_metadata(api_key: &str, video_id: &str) -> Result<VideoMetadata> {
    debug!("fetch_video_metadata: api_key={} video_id={}", api_key, video_id);
    let url = format!(
        "https://www.googleapis.com/youtube/v3/videos?id={}&part=snippet&key={}",
        video_id, api_key
    );

    let response = reqwest::get(&url).await?
        .json::<serde_json::Value>().await?;

    if response["items"].as_array().unwrap_or(&Vec::new()).is_empty() {
        return Err(eyre!("Video metadata not found for video_id={}", video_id));
    }

    let snippet = &response["items"][0]["snippet"];
    Ok(VideoMetadata {
        id: video_id.to_string(),
        title: snippet["title"].as_str().unwrap_or_default().to_string(),
        description: snippet["description"].as_str().unwrap_or_default().to_string(),
        channel: snippet["channelTitle"].as_str().unwrap_or_default().to_string(),
        published_at: snippet["publishedAt"].as_str().unwrap_or_default().to_string(),
        tags: snippet["tags"].as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|tag| tag.as_str())
            .map(String::from)
            .collect(),
    })
}

async fn handle_shorts_url(url: &str, folder: &str, width: usize, height: usize, config: &Config) -> Result<()> {
    debug!("handle_shorts_url: url={} folder={} width={} height={} config={:?}", url, folder, width, height, config);
    let video_id = extract_video_id(url)?;
    let metadata = fetch_video_metadata(&YOUTUBE_API_KEY, &video_id).await?;
    let embed_code = generate_embed_code(&video_id, width, height);
    create_markdown_file(
        &metadata.title,
        &metadata.description,
        &embed_code,
        url,
        &metadata.channel,
        &metadata.tags,
        &config.vault,
        folder,
        &config.frontmatter
    ).await
}

async fn handle_youtube_url(url: &str, folder: &str, width: usize, height: usize, config: &Config) -> Result<()> {
    debug!("handle_youtube_url: url={} folder={} width={} height={} config={:?}", url, folder, width, height, config);
    let video_id = extract_video_id(url)?;
    let metadata = fetch_video_metadata(&YOUTUBE_API_KEY, &video_id).await?;
    let embed_code = generate_embed_code(&video_id, width, height);
    create_markdown_file(
        &metadata.title,
        &metadata.description,
        &embed_code,
        url,
        &metadata.channel,
        &metadata.tags,
        &config.vault,
        folder,
        &config.frontmatter
    ).await
}

async fn handle_weblink_url(url: &str, folder: &str, width: usize, height: usize, config: &Config) -> Result<()> {
    debug!("handle_weblink_url: url={} folder={} width={} height={} config={:?}", url, folder, width, height, config);

    let title = "Some Title";
    let description = "Some Description";
    let author = "Some Author";
    let tags_str = vec!["tag1", "tag2"];
    let tags: Vec<String> = tags_str.iter().map(|s| s.to_string()).collect();
    let embed_code = "";

    create_markdown_file(
        title,
        description,
        embed_code,
        url,
        author,
        &tags,
        &config.vault,
        folder,
        &config.frontmatter
    ).await
}

async fn handle_url(url: &str, config: &Config) -> Result<()> {
    debug!("handle_url: url={} config={:?}", url, config);
    match LinkType::from_url(url, config)? {
        LinkType::Shorts(url, folder, width, height) => handle_shorts_url(&url, &folder, width, height, config).await,
        LinkType::YouTube(url, folder, width, height) => handle_youtube_url(&url, &folder, width, height, config).await,
        LinkType::WebLink(url, folder, width, height) => handle_weblink_url(&url, &folder, width, height, config).await,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(LOG_LEVEL.as_str())).init();
    info!("obsidian-link");
    let args = Cli::parse();
    let config = load_config(args.config)?;

    match args.url {
        Some(url) => handle_url(&url, &config).await,
        None => Err(eyre!("No URL provided")),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn load_test_config() -> Config {
        let config_path = shellexpand::tilde("~/.config/obsidian-link/obsidian-link.yml");
        let config_path = Path::new(config_path.as_ref());

        load_config(config_path.to_path_buf()).expect("Failed to load config")
    }

    #[tokio::test]
    async fn test_youtube_shorts_identification() {
        let config = load_test_config();
        let shorts_urls = vec![
            "https://www.youtube.com/shorts/gGrqPbb6fuM",
            "https://www.youtube.com/shorts/FjkS5rjNq-A",
        ];
        for url in shorts_urls {
            let link_type = LinkType::from_url(url, &config).expect("Failed to identify link type");
            assert!(matches!(link_type, LinkType::Shorts(..))); // Updated to expect Shorts
        }
    }

    #[tokio::test]
    async fn test_youtube_url_identification() {
        let config = load_test_config();

        let urls = vec![
            "https://www.youtube.com/watch?v=y4evLICF8kk",
            "https://www.youtube.com/watch?v=U3HndX2QnSo",
            "https://youtu.be/EkDxsQRbIwoA",
            "https://youtu.be/m7lnIdudEy8?si=VE-14Y1Sk93RdA5u",
        ];

        for url in urls {
            let link_type = LinkType::from_url(url, &config).expect("Failed to identify link type");
            assert!(matches!(link_type, LinkType::YouTube(..)));
        }
    }

    #[tokio::test]
    async fn test_weblink_identification() {
        let config = load_test_config();

        let weblink_urls = vec![
            "https://parrot.ai/",
            "https://pdfgpt.io/",
        ];

        for url in weblink_urls {
            let link_type = LinkType::from_url(url, &config).expect("Failed to identify link type");
            assert!(matches!(link_type, LinkType::WebLink(..)));
        }
    }

    #[tokio::test]
    async fn test_invalid_shorts_url_format() {
        let config = load_test_config();
        let invalid_shorts_url = "https://www.youtube.com/notshorts/gGrqPbb6fuM";
        let link_type = LinkType::from_url(invalid_shorts_url, &config).expect("Failed to identify link type");
        assert!(matches!(link_type, LinkType::WebLink(..)), "Expected a WebLink for invalid Shorts URL format");
    }

    #[tokio::test]
    async fn test_invalid_youtube_url_format() {
        let config = load_test_config();
        let invalid_youtube_url = "https://www.notyoutube.com/watch?v=y4evLICF8kk";
        let link_type = LinkType::from_url(invalid_youtube_url, &config).expect("Failed to identify link type");
        assert!(matches!(link_type, LinkType::WebLink(..)), "Expected a WebLink for invalid YouTube URL format");
    }

    #[tokio::test]
    async fn test_fetch_metadata_nonexistent_video() {
        let non_existent_video_id = "thisdoesnotexist12345";
        let result = fetch_video_metadata(&YOUTUBE_API_KEY, non_existent_video_id).await;
        assert!(result.is_err(), "Expected an error for non-existent video metadata fetch");
    }

    #[test]
    fn test_generate_embed_code_non_integer() {
        let video_id = "y4evLICF8kk";
        let embed_code = generate_embed_code(video_id, 0, 0);
        assert!(embed_code.contains("width=\"0\""), "Embed code should contain width=\"0\"");
        assert!(embed_code.contains("height=\"0\""), "Embed code should contain height=\"0\"");
    }

    #[tokio::test]
    async fn test_create_markdown_special_characters() {
        let title = "Test: Special/Characters?*";
        let description = "A test video.";
        let embed_code = "<iframe...></iframe>"; // Example embed code
        let url = "https://www.example.com";
        let author = "Test Channel";
        let tags = vec![String::from("test")];
        let config = load_test_config();

        let result = create_markdown_file(
            title,
            description,
            &embed_code,
            url,
            author,
            &tags,
            &config.vault,
            "test_folder",
            &config.frontmatter
        ).await;

        assert!(result.is_ok(), "Failed to create markdown file with special characters in title");
    }
}
