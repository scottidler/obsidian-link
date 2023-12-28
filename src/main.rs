#![cfg_attr(debug_assertions, allow(unused_imports, unused_variables, unused_mut, dead_code))]

use regex::Regex;
use clap::{Parser, Args};
use serde::Deserialize;
use std::path::PathBuf;
use eyre::{eyre, Result};
use shellexpand;

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

fn is_youtube_url(url: &str) -> bool {
    // Implement logic to determine if the URL is a regular YouTube link or YouTube Shorts link
    false
}

fn handle_url(url: &str) -> Result<()> {
    match LinkType::from_url(url)? {
        LinkType::YouTube(url) => handle_youtube_url(&url),
        LinkType::WebLink(url) => handle_weblink_url(&url),
    }
}

fn handle_youtube_url(url: &str) -> Result<()> {
    // YouTube URL handling logic
    Ok(())
}

fn handle_weblink_url(url: &str) -> Result<()> {
    // Web link handling logic
    Ok(())
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let config = load_config(args.config)?;
    println!("Config: {:?}", config);

    match args.youtube_url {
        Some(url) => handle_url(&url),
        None => Err(eyre::eyre!("No URL provided")),
    }
}

