use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::write::SimpleFileOptions;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the plugin directory
    #[arg(short, long, default_value = ".")]
    path: PathBuf,
}

#[derive(Deserialize, Debug)]
struct CargoToml {
    package: Package,
}

#[derive(Deserialize, Debug)]
struct Package {
    name: String,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let plugin_dir = args.path.canonicalize()?;
    
    // 1. Parse Cargo.toml to get package name
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
    
    // 2. Build the plugin
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
    
    // 3. Locate compiled .wasm file
    let wasm_file = plugin_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(format!("{}.wasm", plugin_name.replace("-", "_")));
        
    if !wasm_file.exists() {
        eprintln!("Error: Compiled Wasm file not found at {:?}", wasm_file);
        std::process::exit(1);
    }
    
    // 4. Locate manifest file
    let manifest_file = plugin_dir.join(format!("{}.json", plugin_name));
    if !manifest_file.exists() {
        eprintln!("Error: Manifest file {} not found in plugin root.", manifest_file.display());
        std::process::exit(1);
    }
    
    // 5. Create .ito archive
    let output_file = plugin_dir.join(format!("{}.ito", plugin_name));
    println!("Creating package: {:?}", output_file);
    
    let file = File::create(&output_file)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    
    // Write wasm
    zip.start_file(format!("{}.wasm", plugin_name.replace("-", "_")), options)?;
    let mut wasm_content = Vec::new();
    File::open(&wasm_file)?.read_to_end(&mut wasm_content)?;
    zip.write_all(&wasm_content)?;
    
    // Write manifest
    zip.start_file(format!("{}.json", plugin_name), options)?;
    let mut manifest_content = Vec::new();
    File::open(&manifest_file)?.read_to_end(&mut manifest_content)?;
    zip.write_all(&manifest_content)?;
    
    zip.finish()?;
    
    println!("Successfully packaged plugin into {}!", output_file.display());
    Ok(())
}
