#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use std::path::PathBuf;
use std::io::Write;
use std::env;

use regex::Regex;
use clap::{Parser, Args};
use serde::Deserialize;
use eyre::{eyre, Result};
use lazy_static::lazy_static;
use reqwest;
use tokio;
use shellexpand;

lazy_static! {
    static ref YOUTUBE_API_KEY: String = env::var("YOUTUBE_API_KEY").expect("YOUTUBE_API_KEY not set in environment");
    static ref CHATGPT_API_KEY: String = env::var("CHATGPT_API_KEY").expect("CHATGPT_API_KEY not set in environment");
}

#[derive(Deserialize, Debug)]
struct Config {
    vault: PathBuf,
    resolution: String,
    frontmatter: Frontmatter,
}

#[derive(Deserialize, Debug)]
struct Frontmatter {
    date: Option<String>,
    day: Option<String>,
    time: Option<String>,
    tags: Option<Vec<String>>,
    url: Option<String>,
    author: Option<String>,
    // Add other fields as needed
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, value_parser, default_value = "~/.config/obsidian-link/obsidian-link.yml")]
    config: PathBuf,

    #[clap(short, long)]
    youtube_url: Option<String>,

    // Add other command line arguments as needed
}

enum LinkType {
    YouTube(String),
    WebLink(String),
}

impl LinkType {
    fn from_url(url: &str) -> Result<LinkType> {
        let youtube_regex = Regex::new(r#"(youtu\.be/|youtube\.com/(watch\?(.*&)?v=|(embed|v|shorts)/))([^?&">]+)"#)
            .map_err(|e| eyre!("Failed to compile YouTube regex: {}", e))?;

        if youtube_regex.is_match(url) {
            Ok(LinkType::YouTube(url.to_string()))
        } else {
            Ok(LinkType::WebLink(url.to_string()))
        }
    }
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

fn expanduser(path: &str) -> Result<PathBuf> {
    let expanded_path = shellexpand::tilde(path);
    Ok(PathBuf::from(expanded_path.into_owned()))
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

async fn create_markdown_file(metadata: &VideoMetadata, embed_code: &str, vault_path: &PathBuf, frontmatter: &Frontmatter) -> Result<()> {
    let file_name = sanitize_filename(&metadata.title);
    let file_path = vault_path.join("youtube").join(file_name + ".md");

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| eyre!("Failed to create markdown file: {}", e))?;

    let frontmatter_str = format_frontmatter(frontmatter, metadata);
    write!(file, "{}\n{}\n\n## Description\n{}", frontmatter_str, embed_code, metadata.description)
        .map_err(|e| eyre!("Failed to write to markdown file: {}", e))
}

fn format_frontmatter(frontmatter: &Frontmatter, metadata: &VideoMetadata) -> String {
    let mut frontmatter_str = String::from("---\n");

    frontmatter_str += &format!("date: {}\n", frontmatter.date.as_ref().unwrap_or(&current_date()));
    frontmatter_str += &format!("day: {}\n", frontmatter.day.as_ref().unwrap_or(&current_day()));
    frontmatter_str += &format!("time: {}\n", frontmatter.time.as_ref().unwrap_or(&current_time()));

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

fn generate_embed_code(video_id: &str, resolution: &String) -> String {
    format!("<iframe src='https://www.youtube.com/embed/{}' resolution='{}'></iframe>", video_id, resolution)
}

fn current_date() -> String {
    // Implement to return current date in desired format
    "2023-01-01".to_string()
}

fn current_day() -> String {
    // Implement to return current day in desired format
    "Mon".to_string()
}

fn current_time() -> String {
    // Implement to return current time in desired format
    "00:00".to_string()
}

fn sanitize_tag(tag: &str) -> String {
    // Implement tag sanitization logic similar to the Python version
    tag.replace("'", "")
       .replace(" ", "-")
       .to_lowercase()
}

fn sanitize_filename(title: &str) -> String {
    title.replace(&['<', '>', ':', '"', '/', '\\', '|', '?', '*'][..], "-")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let config = load_config(args.config)?;
    println!("Config: {:?}", config);

    match args.youtube_url {
        Some(url) => handle_url(&url, &config).await,
        None => Err(eyre!("No URL provided")),
    }
}

async fn handle_url(url: &str, config: &Config) -> Result<()> {
    match LinkType::from_url(url)? {
        LinkType::YouTube(url) => handle_youtube_url(&url, config).await,
        LinkType::WebLink(url) => handle_weblink_url(&url, config).await,
    }
}

async fn handle_youtube_url(url: &str, config: &Config) -> Result<()> {
    let video_id = extract_video_id(url)?;
    let metadata = fetch_video_metadata(&YOUTUBE_API_KEY, &video_id).await?;
    let embed_code = generate_embed_code(&video_id, &config.resolution);

    create_markdown_file(&metadata, &embed_code, &config.vault, &config.frontmatter).await
}


async fn handle_weblink_url(url: &str, config: &Config) -> Result<()> {
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

    // Parse the response to extract video metadata
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

    #[test]
    fn test_youtube_url_identification() {
        let youtube_urls = vec![
            "https://www.youtube.com/watch?v=y4evLICF8kk",
            "https://youtu.be/EkDxsQRbIwoA",
            "https://www.youtube.com/shorts/gGrqPbb6fuM",
        ];

        for url in youtube_urls {
            match LinkType::from_url(url) {
                Ok(LinkType::YouTube(_)) => (),
                _ => panic!("YouTube URL not identified correctly: {}", url),
            }
        }
    }

    #[test]
    fn test_weblink_url_identification() {
        let web_links = vec![
            "https://parrot.ai/",
            "https://pdfgpt.io/",
            "https://alohahoo.com/products/men's-casual-jacket-paisley-pattern-print-long-sleeve-pockets-jacket",
            "https://greenpointefloorsupply.com/services/",
            "https://phys.org/news/2023-12-theory-einstein-gravity-quantum-mechanics.html",
        ];

        for url in web_links {
            match LinkType::from_url(url) {
                Ok(LinkType::WebLink(_)) => (),
                _ => panic!("WebLink URL not identified correctly: {}", url),
            }
        }
    }
}
