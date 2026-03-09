use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Command;
use zip::write::SimpleFileOptions;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Build and package a plugin into an .ito file
    Pack {
        /// Path to the plugin directory
        #[arg(short, long, default_value = ".")]
        path: PathBuf,
    },
    /// Verify an .ito plugin
    Verify {
        /// Path to the .ito file
        path: PathBuf,
    },
    /// Build a static repository from a folder of .ito files
    Repo {
        /// Path to the directory containing .ito files
        #[arg(short, long, default_value = ".")]
        input: PathBuf,
        /// Path to the output directory for the repository
        #[arg(short, long, default_value = "public")]
        output: PathBuf,
        /// Name of the repository
        #[arg(long, default_value = "Ito Repository")]
        name: String,
        /// URL of the repository
        #[arg(long)]
        url: String,
    },
    /// Serve a directory over HTTP (for dev usage)
    Serve {
        /// Path to the directory to serve
        #[arg(long, default_value = "public")]
        path: PathBuf,
        /// Port to bind to
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
}

#[derive(Deserialize, Debug)]
struct CargoToml {
    package: CargoPackage,
}

#[derive(Deserialize, Debug)]
struct CargoPackage {
    name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PluginManifest {
    id: String,
    name: String,
    version: String, // String instead of Int to match semver
    min_app_version: String,
    url: Option<String>,
    sourceUrl: Option<String>,
    contentRating: Option<i32>,
    nsfw: Option<i32>,
    language: Option<String>,
    languages: Option<Vec<String>>,
    #[serde(rename = "type")]
    plugin_type: String, // "manga" or "anime"
    author: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RepoPackage {
    id: String,
    name: String,
    version: String,
    min_app_version: String,
    download_url: String,
    icon_url: Option<String>,
    sha256: String,
    #[serde(rename = "type")]
    plugin_type: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct RepoIndex {
    repo_name: String,
    repo_url: String,
    description: String,
    packages: Vec<RepoPackage>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Pack { path } => pack_plugin(path),
        Commands::Verify { path } => verify_plugin(path),
        Commands::Repo { input, output, name, url } => build_repo(input, output, name, url),
        Commands::Serve { path, port } => serve_repo(path, port),
    }
}

fn pack_plugin(plugin_dir: PathBuf) -> io::Result<()> {
    let plugin_dir = plugin_dir.canonicalize()?;
    
    let cargo_toml_path = plugin_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        eprintln!("Error: Cargo.toml not found in directory {:?}", plugin_dir);
        std::process::exit(1);
    }
    
    let cargo_toml_content = std::fs::read_to_string(&cargo_toml_path)?;
    let cargo_toml: CargoToml = toml::from_str(&cargo_toml_content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    
    let plugin_name = cargo_toml.package.name;
    println!("Found plugin: {}", plugin_name);
    
    println!("Building plugin...");
    let status = Command::new("cargo")
        .arg("build")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--release")
        .current_dir(&plugin_dir)
        .status()?;
        
    if !status.success() {
        eprintln!("Error: Cargo build failed with status {:?}", status);
        std::process::exit(1);
    }
    
    let wasm_file = plugin_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{}.wasm", plugin_name.replace("-", "_")));
        
    if !wasm_file.exists() {
        eprintln!("Error: Compiled Wasm file not found at {:?}", wasm_file);
        std::process::exit(1);
    }
    
    // Look for manifest.json
    let mut manifest_file = plugin_dir.join("manifest.json");
    if !manifest_file.exists() {
        // Fallback to old name
        manifest_file = plugin_dir.join(format!("{}.json", plugin_name));
        if !manifest_file.exists() {
            eprintln!("Error: manifest.json not found in plugin root.");
            std::process::exit(1);
        }
    }
    
    // Validate manifest
    let manifest_content = std::fs::read_to_string(&manifest_file)?;
    if let Err(e) = serde_json::from_str::<PluginManifest>(&manifest_content) {
        eprintln!("Error: Invalid manifest.json format: {}", e);
        std::process::exit(1);
    }
    
    let output_file = plugin_dir.join(format!("{}.ito", plugin_name));
    println!("Creating package: {:?}", output_file);
    
    let file = File::create(&output_file)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    
    zip.start_file("main.wasm", options)?;
    let mut wasm_content = Vec::new();
    File::open(&wasm_file)?.read_to_end(&mut wasm_content)?;
    zip.write_all(&wasm_content)?;
    
    zip.start_file("manifest.json", options)?;
    zip.write_all(manifest_content.as_bytes())?;
    
    let icon_file = plugin_dir.join("icon.png");
    if icon_file.exists() {
        zip.start_file("icon.png", options)?;
        let mut icon_content = Vec::new();
        File::open(&icon_file)?.read_to_end(&mut icon_content)?;
        zip.write_all(&icon_content)?;
    }
    
    zip.finish()?;
    
    println!("Successfully packaged plugin into {}!", output_file.display());
    Ok(())
}

fn verify_plugin(path: PathBuf) -> io::Result<()> {
    if !path.exists() {
        eprintln!("File not found: {:?}", path);
        std::process::exit(1);
    }
    
    let file = File::open(&path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    
    let mut has_manifest = false;
    let mut has_wasm = false;
    
    for i in 0..archive.len() {
        let file = archive.by_index(i).unwrap();
        match file.name() {
            "manifest.json" => has_manifest = true,
            "main.wasm" => has_wasm = true,
            _ => {}
        }
    }
    
    if !has_manifest || !has_wasm {
        eprintln!("Verification failed: Missing manifest.json or main.wasm in the archive.");
        std::process::exit(1);
    }
    
    // Read manifest
    let mut manifest_file = archive.by_name("manifest.json").unwrap();
    let mut manifest_str = String::new();
    manifest_file.read_to_string(&mut manifest_str)?;
    
    let manifest: PluginManifest = serde_json::from_str(&manifest_str).map_err(|e| {
        eprintln!("Verification failed: Invalid manifest JSON. {}", e);
        io::Error::new(io::ErrorKind::InvalidData, e.to_string())
    })?;
    
    println!("Plugin {} v{} verified successfully.", manifest.name, manifest.version);
    Ok(())
}

fn build_repo(input: PathBuf, output: PathBuf, name: String, url: String) -> io::Result<()> {
    if !output.exists() {
        std::fs::create_dir_all(&output)?;
    }
    
    let packages_dir = output.join("packages");
    let icons_dir = output.join("icons");
    std::fs::create_dir_all(&packages_dir)?;
    std::fs::create_dir_all(&icons_dir)?;
    
    let mut repo_packages = Vec::new();
    
    for entry in std::fs::read_dir(input)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().map_or(false, |ext| ext == "ito") {
            println!("Processing {:?}", path);
            
            let mut file = File::open(&path)?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            
            // Hash the .ito file
            let mut hasher = Sha256::new();
            hasher.update(&buffer);
            let result = hasher.finalize();
            let sha256_hash = hex::encode(result);
            
            // Extract manifest and icon
            let file_for_zip = File::open(&path)?;
            let mut archive = match zip::ZipArchive::new(file_for_zip) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("Warning: Could not read {:?} as ZIP.", path);
                    continue;
                }
            };
            
            let manifest: PluginManifest = {
                let mut manifest_file = match archive.by_name("manifest.json") {
                    Ok(f) => f,
                    Err(_) => {
                        eprintln!("Warning: manifest.json not found in {:?}", path);
                        continue;
                    }
                };
                let mut s = String::new();
                manifest_file.read_to_string(&mut s)?;
                match serde_json::from_str(&s) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("Warning: Invalid manifest in {:?}: {}", path, e);
                        continue;
                    }
                }
            };
            
            let pkg_filename = format!("{}-v{}.ito", manifest.id, manifest.version);
            let dest_pkg_path = packages_dir.join(&pkg_filename);
            std::fs::copy(&path, &dest_pkg_path)?;
            
            let mut icon_url = None;
            if let Ok(mut icon_file) = archive.by_name("icon.png") {
                let icon_filename = format!("{}-v{}.png", manifest.id, manifest.version);
                let dest_icon_path = icons_dir.join(&icon_filename);
                let mut out_icon = File::create(&dest_icon_path)?;
                io::copy(&mut icon_file, &mut out_icon)?;
                icon_url = Some(format!("icons/{}", icon_filename));
            }
            
            repo_packages.push(RepoPackage {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                min_app_version: manifest.min_app_version,
                download_url: format!("packages/{}", pkg_filename),
                icon_url,
                sha256: sha256_hash,
                plugin_type: manifest.plugin_type,
            });
        }
    }
    
    let index = RepoIndex {
        repo_name: name,
        repo_url: url,
        description: "Official verified plugins for Ito.".to_string(),
        packages: repo_packages,
    };
    
    let index_path = output.join("index.json");
    let mut index_file = File::create(&index_path)?;
    let index_json = serde_json::to_string_pretty(&index)?;
    index_file.write_all(index_json.as_bytes())?;
    
    let min_index_path = output.join("index.min.json");
    let mut min_index_file = File::create(&min_index_path)?;
    let min_index_json = serde_json::to_string(&index)?;
    min_index_file.write_all(min_index_json.as_bytes())?;
    
    println!("Repository built successfully at {:?}", output);
    Ok(())
}

fn serve_repo(path: PathBuf, port: u16) -> io::Result<()> {
    if !path.exists() {
        eprintln!("Error: Path {:?} does not exist.", path);
        std::process::exit(1);
    }

    let addr = format!("0.0.0.0:{}", port);
    println!("Serving directory {:?} on http://{}", path, addr);

    rouille::start_server(addr, move |request| {
        let response = rouille::match_assets(request, &path);
        if response.is_success() {
            response
        } else {
            rouille::Response::html("404 Not Found").with_status_code(404)
        }
    });
}
