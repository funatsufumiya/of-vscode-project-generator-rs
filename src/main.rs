use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Write, BufRead, BufReader};
use std::process;
use std::collections::HashSet;
// use std::sync::atomic::{AtomicBool, Ordering};
// use std::sync::Mutex;
use argparse::{ArgumentParser, Store, StoreTrue};
// use once_cell::sync::Lazy;
use serde_json::{json, Value};
use log::{info, warn, debug};

const VERSION: &str = "0.1.0";
const MAC_SDK_ROOT: &str = "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk";

// static GLOBAL_CONFIG: Lazy<Mutex<Config>> = Lazy::new(|| Mutex::new(Config::default()));

#[derive(Default)]
struct Config {
    path: String,
    // debug: bool,
    show_version: bool,
    ignore_excludes: bool,
}

struct ExcludePattern {
    pattern: PathBuf,
    has_wildcard: bool,      // % で終わるか
    has_dir_wildcard: bool,  // /% で終わるか
}

impl ExcludePattern {
    fn pattern_str(&self) -> String {
        let mut pattern = self.pattern.to_string_lossy().to_string();
        if self.has_dir_wildcard {
            pattern.push_str("/%");
        } else if self.has_wildcard {
            pattern.push('%');
        }
        pattern
    }
}

#[derive(Debug, PartialEq)]
enum OS {
    Mac,
    Linux,
    Windows,
    Unknown,
}

impl OS {
    fn to_str(&self) -> &'static str {
        match self {
            OS::Mac => "Mac",
            OS::Linux => "Linux",
            OS::Windows => "Win32",
            OS::Unknown => "Unknown",
        }
    }

    fn current() -> Self {
        if cfg!(target_os = "macos") {
            OS::Mac
        } else if cfg!(target_os = "linux") {
            OS::Linux
        } else if cfg!(target_os = "windows") {
            OS::Windows
        } else {
            OS::Unknown
        }
    }
}

fn main() -> io::Result<()> {
    let mut config = Config::default();
    
    {
        let desc = format!("openFrameworks VSCode Project Generator {}", VERSION);
        let mut parser = ArgumentParser::new();
        parser.set_description(&desc);
        parser.refer(&mut config.path)
            .add_argument("path", Store, "Project path")
            .required();
        // parser.refer(&mut config.debug)
        //     .add_option(&["-d", "--debug"], StoreTrue, "Enable debug output");
        parser.refer(&mut config.show_version)
            .add_option(&["-v", "--version"], StoreTrue, "Show version");
        parser.refer(&mut config.ignore_excludes)
            .add_option(&["-i", "--ignore-excludes"], StoreTrue, "Ignore excludes");
        parser.parse_args_or_exit();
    }

    env_logger::init();

    if config.show_version {
        print!("of-vscode-project-generator-rs {}", VERSION);
        process::exit(0);
    }

    // {
    //     let mut g = GLOBAL_CONFIG.lock().unwrap();
    //     g.debug = config.debug;
    //     g.path = config.path.clone();
    //     g.ignore_excludes = config.ignore_excludes;
    // }

    println!("\n======================================");
    println!("   of-vscode-project-generator-rs v{}", VERSION);
    println!("======================================\n");

    let proj_path = PathBuf::from(&config.path);
    if !proj_path.exists() {
        eprintln!("Error: Project path does not exist");
        process::exit(1);
    }

    // Change current directory to project path
    std::env::set_current_dir(&proj_path)?;
    
    let proj_path = std::fs::canonicalize(&proj_path)?;

    let os = OS::current();

    println!("------\n");
    if config.ignore_excludes {
        println!("[Info] Ignoring excludes by user (-i / --ignore-excludes) !!!");
    }
    
    println!("[Info] OS: {}", os.to_str());
    println!("[Info] project path: '{}'", proj_path.display());

    // Project validation
    if !proj_path.join("src").join("ofApp.h").exists() {
        println!("[Warning] This directory seems not to be valid app path!");
        print!("  Are you sure to proceed? (Y/n): ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim() != "Y" {
            println!("cancelled.");
            process::exit(1);
        }
    }

    // Check for existing .vscode/c_cpp_properties.json
    if proj_path.join(".vscode").join("c_cpp_properties.json").exists() {
        println!("[Warning] '.vscode/c_cpp_properties.json' already exists!");
        print!("  Are you sure to proceed? (Y/n): ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim() != "Y" {
            println!("cancelled.");
            process::exit(1);
        }
    }

    // Validate OF root directory
    let of_root = resolve_path(proj_path.join("../../..").as_path());

    if !of_root.join("apps").exists() {
        eprintln!("[Error] '{}' is not OF root. Stops.", of_root.display());
        process::exit(1);
    }

    // Generate include paths
    let mut include_paths = HashSet::new();
    
    // Add basic paths
    include_paths.insert("${workspaceFolder}/**".to_string());
    include_paths.insert("${workspaceFolder}/src".to_string());
    include_paths.insert("${workspaceFolder}/src/**".to_string());

    // Add OF paths
    include_paths.insert(of_root.join("libs/openFrameworks").to_string_lossy().to_string());
    include_paths.insert(format!("{}/libs/openFrameworks/**", of_root.display()));
    
    // Add library paths
    let libs_path = of_root.join("libs");
    if libs_path.exists() {
        for entry in fs::read_dir(libs_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.file_name().unwrap() != "openFrameworks" {
                if path.join("include").exists() {
                    let include_path = path.join("include");
                    include_paths.insert(include_path.to_string_lossy().to_string());
                    include_paths.insert(format!("{}/**", include_path.display()));
                }
            }
        }
    }

    // Process addons.make
    let addons_path = proj_path.join("addons.make");
    if addons_path.exists() {
        println!("[Info] Reading addons.make");
        let file = File::open(addons_path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let addon = line?.trim().to_string();
            if addon.is_empty() {
                continue;
            }

            let addon_path_raw = if addon.starts_with("addons/") {
                proj_path.join(&addon)
            } else {
                of_root.join("addons").join(&addon)
            };

            let addon_path = std::fs::canonicalize(&addon_path_raw)
                .unwrap_or_else(|_| {
                    eprintln!("[Error] '{}' doesn't exist. Stops.", addon_path_raw.display());
                    process::exit(1);
                });

            println!("[Info] Checking '{}'", addon_path.display());

            let excludes = if !config.ignore_excludes {
                parse_addon_excludes(&addon_path)
            } else {
                Vec::new()
            };

            // Add src directories
            if addon_path.join("src").exists() {
                add_directories_recursively(&addon_path.join("src"), &excludes, &mut include_paths)?;
            }

            // Add libs directories
            let libs_path = addon_path.join("libs");
            if libs_path.exists() {
                for entry in fs::read_dir(libs_path)? {
                    let entry = entry?;
                    let lib_path = entry.path();
                    if lib_path.is_dir() {
                        if lib_path.join("src").exists() {
                            add_directories_recursively(&lib_path.join("src"), &excludes, &mut include_paths)?;
                        }
                        if lib_path.join("include").exists() {
                            include_paths.insert(lib_path.join("include").to_string_lossy().to_string());
                            add_directories_recursively(&lib_path.join("include"), &excludes, &mut include_paths)?;
                        }
                    }
                }
            }
        }
    }

    // Add Mac SDK paths
    let mut mac_framework_paths = Vec::new();
    if os == OS::Mac {
        let sdk_path = PathBuf::from(MAC_SDK_ROOT);
        if sdk_path.exists() {
            include_paths.insert(format!("{}/usr/include", MAC_SDK_ROOT));
            mac_framework_paths.push(format!("{}/System/Library/Frameworks", MAC_SDK_ROOT));
        }
    }

    // Determine architecture
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };

    let mut include_paths_final = include_paths.iter()
        .map(|s| resolve_path_or_ignore(s))
        .collect::<Vec<_>>();
    include_paths_final.sort();

    // Generate c_cpp_properties.json
    let config = json!({
        "configurations": [{
            "name": os.to_str(),
            "includePath": include_paths_final,
            "defines": [],
            "macFrameworkPath": mac_framework_paths,
            "cStandard": "c11",
            "cppStandard": "c++17",
            "intelliSenseMode": format!("clang-{}", arch)
        }],
        "version": 4
    });

    // Save configuration
    fs::create_dir_all(proj_path.join(".vscode"))?;
    let config_path = proj_path.join(".vscode/c_cpp_properties.json");
    let mut file = File::create(config_path)?;
    write!(file, "{}", serde_json::to_string_pretty(&config)?)?;

    println!("[Info] Done!");
    println!("[Info] Saved to '{}/.vscode/c_cpp_properties.json' :)", proj_path.display());

    Ok(())
}

fn add_directories_recursively(
    dir: &Path,
    excludes: &[ExcludePattern],
    include_paths: &mut HashSet<String>
) -> io::Result<()> {
    let dir_str = dir.to_string_lossy().to_string();
    if !is_excluded_dir(dir, excludes) {
        include_paths.insert(dir_str);
    }
    
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                add_directories_recursively(&path, excludes, include_paths)?;
            }
        }
    }
    Ok(())
}

fn parse_addon_excludes(addon_path: &Path) -> Vec<ExcludePattern> {
    let config_path = addon_path.join("addon_config.mk");
    let mut excludes = Vec::new();

    if !config_path.exists() {
        return excludes;
    }

    let file = File::open(config_path).unwrap();
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.unwrap();
        let line = line.trim();
        
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with("ADDON_SOURCES_EXCLUDE") || line.starts_with("ADDON_INCLUDES_EXCLUDE") {
            let parts: Vec<&str> = line.split(['=', '+']).collect();
            if parts.len() >= 2 {
                let pattern = parts.last().unwrap().trim();
                if !pattern.is_empty() {
                    let has_dir_wildcard = pattern.ends_with("/%");
                    let has_wildcard = pattern.ends_with('%');
                    let clean_pattern = pattern.trim_end_matches("/%").trim_end_matches('%');
                    excludes.push(ExcludePattern {
                        pattern: addon_path.join(clean_pattern),
                        has_wildcard,
                        has_dir_wildcard,
                    });
                }
            }
        }
    }
    
    debug!("excludes: {}", excludes.iter()
        .map(|e| e.pattern_str())
        .collect::<Vec<_>>()
        .join(", "));
    excludes
}

fn is_excluded_dir(dir_abs: &Path, excludes: &[ExcludePattern]) -> bool {
    let dir_str = dir_abs.to_string_lossy();
    
    for exclude in excludes {
        let exclude_str = exclude.pattern.to_string_lossy();
        
        if exclude.has_dir_wildcard {
            // /% パターン: ディレクトリとその配下すべてを除外
            let prefix = exclude_str.as_ref();
            if dir_str.starts_with(prefix) {
                info!("excluded {} (dir wildcard {}/%)", dir_str, prefix);
                return true;
            }
        }
        else if exclude.has_wildcard {
            // % パターン: プレフィックスマッチ
            let prefix = exclude_str.as_ref();
            if dir_str.starts_with(prefix) {
                info!("excluded {} (wildcard {}%)", dir_str, prefix);
                return true;
            }
        }
        else {
            // 通常パターン: 完全一致
            if dir_str == exclude_str {
                info!("excluded {} (exact match {})", dir_str, exclude_str);
                return true;
            }
        }
    }
    
    debug!("not excluded {:?}", dir_abs);
    false
}

fn resolve_path(path: &Path) -> PathBuf {
    if path.exists() {
        std::fs::canonicalize(path).unwrap()
    } else {
        eprintln!("[Error] '{}' doesn't exist. Stops.", path.display());
        process::exit(1);
    }
}

fn resolve_path_or_ignore(path_str: &str) -> String {
    let path = Path::new(path_str);
    if path.exists() {
        std::fs::canonicalize(path).unwrap().to_str().unwrap().to_string()
    } else {
        path_str.to_string()
    }
}