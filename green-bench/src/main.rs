use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::Deserialize;
use std::error::Error;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

const DEFAULT_DATASETS: &[&str] = &[
    "livebench/math",
    "livebench/reasoning",
    "livebench/language",
    "livebench/instruction_following",
];

#[derive(Parser)]
#[command(
    name = "greenbench-live",
    version,
    about = "GreenBench LiveBench-lite CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download LiveBench datasets from Hugging Face
    Download(DownloadArgs),
}

#[derive(Parser)]
struct DownloadArgs {
    /// Dataset ids to download (e.g. livebench/math). If omitted, downloads a default set.
    #[arg(short, long)]
    dataset: Vec<String>,
    /// Root directory to place downloaded datasets
    #[arg(short, long, default_value = "data")]
    output_dir: PathBuf,
    /// Also download README.md files
    #[arg(long, default_value_t = false)]
    include_readme: bool,
}

#[derive(Debug, Deserialize)]
struct DatasetInfo {
    siblings: Vec<Sibling>,
}

#[derive(Debug, Deserialize)]
struct Sibling {
    rfilename: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Download(args) => run_download(args).await?,
    }
    Ok(())
}

async fn run_download(args: DownloadArgs) -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    let dataset_ids = if args.dataset.is_empty() {
        DEFAULT_DATASETS.iter().map(|s| s.to_string()).collect()
    } else {
        args.dataset
    };

    for dataset_id in dataset_ids {
        download_dataset(&client, &dataset_id, &args.output_dir, args.include_readme).await?;
    }
    Ok(())
}

async fn download_dataset(
    client: &Client,
    dataset_id: &str,
    output_root: &Path,
    include_readme: bool,
) -> Result<(), Box<dyn Error>> {
    println!("Fetching metadata for {dataset_id}...");
    let meta_url = format!("https://huggingface.co/api/datasets/{dataset_id}");
    let info: DatasetInfo = client
        .get(meta_url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let files: Vec<String> = info
        .siblings
        .into_iter()
        .filter(|s| {
            s.rfilename.starts_with("data/")
                || (include_readme && s.rfilename.eq_ignore_ascii_case("README.md"))
        })
        .filter(|s| s.rfilename != ".gitattributes")
        .map(|s| s.rfilename)
        .collect();

    if files.is_empty() {
        println!("No downloadable files found for {dataset_id}.");
        return Ok(());
    }

    for filename in files {
        download_file(client, dataset_id, &filename, output_root).await?;
    }

    Ok(())
}

async fn download_file(
    client: &Client,
    dataset_id: &str,
    filename: &str,
    output_root: &Path,
) -> Result<(), Box<dyn Error>> {
    let url = format!("https://huggingface.co/datasets/{dataset_id}/resolve/main/{filename}");
    let dest_path = output_root.join(dataset_id).join(filename);

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    println!("Downloading {url} -> {}", dest_path.display());
    let mut resp = client.get(url).send().await?.error_for_status()?;
    let mut file = fs::File::create(&dest_path).await?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await?;
    }
    Ok(())
}
