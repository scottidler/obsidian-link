#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use std::path::PathBuf;
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

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, value_parser, default_value = "~/.config/obsidian-link/obsidian-link.yml")]
    config: PathBuf,

    #[clap(short, long)]
    youtube_url: Option<String>,
}

enum LinkType {
    YouTube(String, String, usize, usize),
    WebLink(String, String, usize, usize),
}

impl LinkType {
    fn from_url(url: &str, config: &Config) -> Result<LinkType> {
        for link in &config.links {
            println!("Testing regex: {}", &link.regex); // Print the regex being tested
            let regex = Regex::new(&link.regex)
                .map_err(|e| eyre!("Failed to compile regex for {}: {}", link.name, e))?;

            if regex.is_match(url) {
                println!("URL matched regex: {}", &link.regex); // Print when a URL matches
                let (width, height) = get_resolution(&link.name, config)
                    .ok_or_else(|| eyre!("Failed to find resolution for {}", link.name))?;

                return Ok(match link.name.as_str() {
                    "youtube" | "shorts" => LinkType::YouTube(url.to_string(), link.folder.clone(), width, height),
                    _ => LinkType::WebLink(url.to_string(), link.folder.clone(), width, height),
                });
            } else {
                println!("URL did not match regex: {}", &link.regex); // Print when a URL does not match
            }
        }

        Err(eyre!("No matching link type found for URL"))
    }
}

fn get_resolution(link_name: &str, config: &Config) -> Option<(usize, usize)> {
    config.links.iter().find(|link| link.name == link_name)
        .and_then(|link| {
            match link_name {
                "shorts" => {
                    SHORTS_RESOLUTIONS.get(link.resolution.as_str())
                },
                "youtube" | "weblink" => {
                    RESOLUTIONS.get(link.resolution.as_str())
                },
                _ => None, // Or handle other types
            }
        }).copied()
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

fn expanduser<T: AsRef<str>>(path: T) -> Result<PathBuf> {
    let expanded_path_str = shellexpand::tilde(path.as_ref());
    Ok(PathBuf::from(expanded_path_str.into_owned()))
}

fn load_config(config_path: PathBuf) -> Result<Config> {
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
    let pattern = Regex::new(r#"(youtu\.be/|youtube\.com/(watch\?(.*&)?v=|(embed|v|shorts)/))([^?&">]+)"#)
        .map_err(|e| eyre!("Failed to compile regex: {}", e))?;

    pattern.captures(url)
        .and_then(|caps| caps.get(5))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| eyre!("Failed to extract video ID from URL"))
}

async fn create_markdown_file(metadata: &VideoMetadata, embed_code: &str, vault_path: &PathBuf, folder: &str, frontmatter: &Frontmatter) -> Result<()> {
    let vault_path_str = vault_path.to_str()
        .ok_or_else(|| eyre!("Failed to convert vault path to string"))?;
    let vault_path_expanded = expanduser(vault_path_str)?;
    let full_path = vault_path_expanded.join(folder);
    let file_name = sanitize_filename(&metadata.title);
    let file_path = full_path.join(file_name + ".md");

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| eyre!("Failed to create markdown file: {:?} with error {}", file_path, e))?;

    let frontmatter_str = format_frontmatter(frontmatter, metadata);
    write!(file, "{}\n{}\n\n## Description\n{}", frontmatter_str, embed_code, metadata.description)
        .map_err(|e| eyre!("Failed to write to markdown file: {}", e))
}

fn format_frontmatter(frontmatter: &Frontmatter, metadata: &VideoMetadata) -> String {
    let mut frontmatter_str = String::from("---\n");

    let (current_date, current_day, current_time) = today();
    frontmatter_str += &format!("date: {}\n", frontmatter.date.as_ref().unwrap_or(&current_date));
    frontmatter_str += &format!("day: {}\n", frontmatter.day.as_ref().unwrap_or(&current_day));
    frontmatter_str += &format!("time: {}\n", frontmatter.time.as_ref().unwrap_or(&current_time));

    let tags = frontmatter.tags.as_ref().unwrap_or(&metadata.tags);
    if !tags.is_empty() {
        frontmatter_str += "tags:\n";
        for tag in tags {
            frontmatter_str += &format!("  - {}\n", sanitize_tag(tag));
        }
    }

    if let Some(url) = &frontmatter.url {
        frontmatter_str += &format!("url: {}\n", url);
    } else {
        frontmatter_str += &format!("url: https://www.youtube.com/watch?v={}\n", metadata.id);
    }
    frontmatter_str += &format!("author: {}\n", frontmatter.author.as_ref().unwrap_or(&metadata.channel));

    frontmatter_str += "---\n\n";
    frontmatter_str
}

fn generate_embed_code(video_id: &str, width: usize, height: usize) -> String {
    format!(
        "<iframe width=\"{}\" height=\"{}\" src=\"https://www.youtube.com/embed/{}\" frameborder=\"0\" allowfullscreen></iframe>",
        width, height, video_id
    )
}


fn today() -> (String, String, String) {
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

fn sanitize_tag(tag: &str) -> String {
    tag.replace("'", "")
       .chars()
       .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { '-' })
       .collect::<String>()
       .replace(' ', "-")
       .to_lowercase()
}

fn sanitize_filename(title: &str) -> String {
    title.replace(&['<', '>', ':', '"', '/', '\\', '|', '?', '*'][..], "-")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let config = load_config(args.config)?;

    match args.youtube_url {
        Some(url) => handle_url(&url, &config).await,
        None => Err(eyre!("No URL provided")),
    }
}

async fn handle_url(url: &str, config: &Config) -> Result<()> {
    match LinkType::from_url(url, config)? {
        LinkType::YouTube(url, folder, width, height) => handle_youtube_url(&url, &folder, width, height, config).await,
        LinkType::WebLink(url, folder, width, height) => handle_weblink_url(&url, &folder, width, height, config).await,
    }
}

async fn handle_youtube_url(url: &str, folder: &str, width: usize, height: usize, config: &Config) -> Result<()> {
    let video_id = extract_video_id(url)?;
    let metadata = fetch_video_metadata(&YOUTUBE_API_KEY, &video_id).await?;
    let embed_code = generate_embed_code(&video_id, width, height);
    create_markdown_file(&metadata, &embed_code, &config.vault, folder, &config.frontmatter).await
}

async fn handle_weblink_url(url: &str, folder: &str, width: usize, height: usize, config: &Config) -> Result<()> {
    // Web link handling logic
    Ok(())
}

async fn fetch_video_metadata(api_key: &str, video_id: &str) -> Result<VideoMetadata> {
    let url = format!(
        "https://www.googleapis.com/youtube/v3/videos?id={}&part=snippet&key={}",
        video_id, api_key
    );

    let response = reqwest::get(&url).await?
        .json::<serde_json::Value>().await?;

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


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_youtube_url_identification() {
        let config = load_config(PathBuf::from("~/.config/obsidian-link/obsidian-link.yml")).unwrap();

        let youtube_urls = vec![
            "https://www.youtube.com/watch?v=y4evLICF8kk",
            "https://www.youtube.com/watch?v=U3HndX2QnSo",
            "https://youtu.be/EkDxsQRbIwoA",
            "https://youtu.be/m7lnIdudEy8?si=VE-14Y1Sk93RdA5u",
        ];

        for url in youtube_urls {
            let link_type = LinkType::from_url(url, &config).unwrap();
            assert!(matches!(link_type, LinkType::YouTube(..)));
        }
    }

    #[tokio::test]
    async fn test_youtube_shorts_identification() {
        let config = load_config(PathBuf::from("~/.config/obsidian-link/obsidian-link.yml")).unwrap();

        let shorts_urls = vec![
            "https://www.youtube.com/shorts/gGrqPbb6fuM",
            "https://www.youtube.com/shorts/FjkS5rjNq-A",
        ];

        for url in shorts_urls {
            let link_type = LinkType::from_url(url, &config).unwrap();
            assert!(matches!(link_type, LinkType::YouTube(..)));
        }
    }

    #[tokio::test]
    async fn test_weblink_identification() {
        let config = load_config(PathBuf::from("~/.config/obsidian-link/obsidian-link.yml")).unwrap();

        let weblink_urls = vec![
            "https://parrot.ai/",
            "https://pdfgpt.io/",
            // Add other weblink URLs here
        ];

        for url in weblink_urls {
            let link_type = LinkType::from_url(url, &config).unwrap();
            assert!(matches!(link_type, LinkType::WebLink(..)));
        }
    }
}

