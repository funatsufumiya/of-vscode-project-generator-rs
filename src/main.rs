use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process;

use argparse::{ArgumentParser, Store, StoreTrue};
use log::{debug, info, warn};
use serde_json::json;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAC_SDK_ROOT: &str =
    "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk";

#[derive(Default)]
struct Config {
    path: String,
    show_version: bool,
    ignore_excludes: bool,
    yes: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExcludePattern {
    pub pattern: PathBuf,
    pub has_wildcard: bool,
    pub has_dir_wildcard: bool,
}

impl ExcludePattern {
    pub fn pattern_str(&self) -> String {
        let mut pattern = self.pattern.to_string_lossy().to_string();
        if self.has_dir_wildcard {
            pattern.push_str("/%");
        } else if self.has_wildcard {
            pattern.push('%');
        }
        pattern
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum OS {
    Mac,
    Linux,
    Windows,
    Unknown,
}

impl OS {
    pub fn to_str(&self) -> &'static str {
        match self {
            OS::Mac => "Mac",
            OS::Linux => "Linux",
            OS::Windows => "Win32",
            OS::Unknown => "Unknown",
        }
    }

    pub fn current() -> Self {
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
        let desc = format!(
            "openFrameworks VSCode Project Generator (for static analysis only) v{}",
            VERSION
        );
        let mut parser = ArgumentParser::new();
        parser.set_description(&desc);
        parser
            .refer(&mut config.path)
            .add_argument("path", Store, "Project path")
            .required();
        parser
            .refer(&mut config.show_version)
            .add_option(&["-v", "--version"], StoreTrue, "Show version");
        parser
            .refer(&mut config.ignore_excludes)
            .add_option(&["-i", "--ignore-excludes"], StoreTrue, "Ignore excludes");
        parser
            .refer(&mut config.yes)
            .add_option(&["-y", "--yes"], StoreTrue, "Skip confirmation prompts");
        parser.parse_args_or_exit();
    }

    env_logger::init();

    if config.show_version {
        println!("of-vscode-project-generator-rs v{}", VERSION);
        process::exit(0);
    }

    println!("\n============================================");
    println!("   of-vscode-project-generator-rs v{}", VERSION);
    println!("============================================\n");

    let proj_path = PathBuf::from(&config.path);
    if !proj_path.exists() {
        eprintln!("Error: Project path does not exist");
        process::exit(1);
    }

    let proj_path = std::fs::canonicalize(&proj_path)?;
    let os = OS::current();

    println!("------\n");
    if config.ignore_excludes {
        println!("[Info] Ignoring excludes by user (-i / --ignore-excludes) !!!");
    }

    println!("[Info] OS: {}", os.to_str());
    println!(
        "[Info] Project path: '{}'",
        normalize_windows_path(proj_path.to_str().unwrap())
    );

    // Project validation
    if !proj_path.join("src").join("ofApp.h").exists() {
        println!("[Warning] This directory seems not to be a valid openFrameworks app path!");
        if !config.yes {
            print!("  Are you sure to proceed? (Y/n): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim() != "Y" {
                println!("cancelled.");
                process::exit(1);
            }
        }
    }

    // Check for existing .vscode/c_cpp_properties.json
    if proj_path.join(".vscode").join("c_cpp_properties.json").exists() {
        println!("[Warning] '.vscode/c_cpp_properties.json' already exists in project root!");
        if !config.yes {
            print!("  Are you sure to proceed? (Y/n): ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim() != "Y" {
                println!("cancelled.");
                process::exit(1);
            }
        }
    }

    // Validate OF root directory (assumes standard 3-level deep directory structure)
    let of_root = resolve_path(proj_path.join("../../..").as_path());
    if !of_root.join("apps").exists() {
        eprintln!("[Error] '{}' is not OF root. Stops.", of_root.display());
        process::exit(1);
    }

    // Collect include paths
    let include_paths =
        collect_include_directories(&proj_path, &of_root, os, config.ignore_excludes)?;
    println!("[Info] Collected {} include directories.", include_paths.len());

    // Generate c_cpp_properties.json
    fs::create_dir_all(proj_path.join(".vscode"))?;
    let config_path = proj_path.join(".vscode").join("c_cpp_properties.json");
    let mut file = File::create(config_path)?;
    let content = generate_c_cpp_properties(&include_paths, os);
    write!(file, "{}", content)?;

    println!(
        "[Info] Generated '{}/.vscode/c_cpp_properties.json'",
        normalize_windows_path(proj_path.to_str().unwrap())
    );
    println!("\n[Success] Project configuration for VSCode completed successfully! :)\n");

    Ok(())
}

pub fn collect_include_directories(
    proj_path: &Path,
    of_root: &Path,
    os: OS,
    ignore_excludes: bool,
) -> io::Result<Vec<String>> {
    let mut include_paths = HashSet::new();

    // 1. Basic project paths
    include_paths.insert("${workspaceFolder}/**".to_string());
    include_paths.insert("${workspaceFolder}/src".to_string());
    include_paths.insert("${workspaceFolder}/src/**".to_string());

    let src_path = proj_path.join("src");
    if src_path.exists() {
        add_directories_recursively(&src_path, &[], &mut include_paths)?;
    }

    // 2. OpenFrameworks core headers
    let of_libs_path = of_root.join("libs").join("openFrameworks");
    if of_libs_path.exists() {
        include_paths.insert(normalize_windows_path(of_libs_path.to_str().unwrap()));
        add_directories_recursively(&of_libs_path, &[], &mut include_paths)?;
    }

    // 3. OpenFrameworks 3rd-party libraries
    let libs_path = of_root.join("libs");
    if libs_path.exists() {
        for entry in fs::read_dir(libs_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.file_name().unwrap() != "openFrameworks" {
                let inc = path.join("include");
                if inc.exists() {
                    add_directories_recursively(&inc, &[], &mut include_paths)?;
                }
            }
        }
    }

    // 4. Process addons.make
    let addons_path = proj_path.join("addons.make");
    if addons_path.exists() {
        println!("[Info] Reading addons.make");
        let file = File::open(addons_path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let addon = line?.trim().to_string();
            if addon.is_empty() || addon.starts_with('#') {
                continue;
            }

            let addon_clean = addon.replace('\\', "/");
            let addon_name = addon_clean.strip_prefix("addons/").unwrap_or(&addon_clean);

            let possible_paths = [
                proj_path.join(&addon),
                proj_path.join(&addon_clean),
                proj_path.join("addons").join(addon_name),
                of_root.join("addons").join(addon_name),
                proj_path.join(addon_name),
            ];

            let mut addon_path_opt = None;
            for p in &possible_paths {
                if p.exists() {
                    if let Ok(canon) = std::fs::canonicalize(p) {
                        addon_path_opt = Some(canon);
                        break;
                    }
                }
            }

            let addon_path = match addon_path_opt {
                Some(p) => p,
                None => {
                    warn!(
                        "[Warning] Addon '{}' not found in project or OF root addons directory. Skipping.",
                        addon
                    );
                    continue;
                }
            };

            println!(
                "[Info] Checking addon '{}'",
                normalize_windows_path(addon_path.to_str().unwrap())
            );

            let excludes = if !ignore_excludes {
                parse_addon_excludes(&addon_path, os)
            } else {
                Vec::new()
            };

            // Add addon root if it contains header files directly
            if dir_has_headers(&addon_path) && !is_excluded_dir(&addon_path, &excludes) {
                include_paths.insert(normalize_windows_path(addon_path.to_str().unwrap()));
            }

            // Add explicit ADDON_INCLUDES from addon_config.mk
            let addon_explicit_includes = parse_addon_includes(&addon_path, os);
            for inc_dir in addon_explicit_includes {
                add_directories_recursively(&inc_dir, &excludes, &mut include_paths)?;
            }

            // Add addon src directories
            let addon_src = addon_path.join("src");
            if addon_src.exists() {
                add_directories_recursively(&addon_src, &excludes, &mut include_paths)?;
            }

            // Add addon libs directories
            let addon_libs = addon_path.join("libs");
            if addon_libs.exists() {
                for entry in fs::read_dir(addon_libs)? {
                    let entry = entry?;
                    let lib_path = entry.path();
                    if lib_path.is_dir() {
                        let lib_src = lib_path.join("src");
                        let lib_include = lib_path.join("include");
                        let has_src = lib_src.exists();
                        let has_include = lib_include.exists();

                        if has_src {
                            add_directories_recursively(&lib_src, &excludes, &mut include_paths)?;
                        }
                        if has_include {
                            add_directories_recursively(&lib_include, &excludes, &mut include_paths)?;
                        }

                        // For libs that directly contain headers/sources without src/ or include/ subdirs (e.g. ofxNDI/libs/utils)
                        if !has_src && !has_include {
                            add_directories_recursively(&lib_path, &excludes, &mut include_paths)?;
                        }
                    }
                }
            }
        }
    }

    // 5. System SDK paths (macOS & Windows)
    if os == OS::Mac {
        let sdk_path = PathBuf::from(MAC_SDK_ROOT);
        if sdk_path.exists() {
            include_paths.insert(format!("{}/usr/include", MAC_SDK_ROOT));
        }
    } else if os == OS::Windows {
        let win_sys_includes = collect_windows_system_include_dirs();
        if !win_sys_includes.is_empty() {
            info!("Found {} Windows system include path(s).", win_sys_includes.len());
            for dir in win_sys_includes {
                include_paths.insert(dir);
            }
        }
    }

    let mut result: Vec<String> = include_paths.into_iter().collect();
    result.sort();
    Ok(result)
}

pub fn find_windows_compiler_path() -> Option<String> {
    let vswhere_path = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let vs_install_path = if Path::new(vswhere_path).exists() {
        match process::Command::new(vswhere_path)
            .args(["-latest", "-property", "installationPath"])
            .output()
        {
            Ok(output) if output.status.success() => {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    Some(PathBuf::from(path_str))
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        None
    };

    let fallback_vs_paths = [
        r"C:\Program Files\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Professional",
    ];

    let mut found_vs_path = vs_install_path;
    if found_vs_path.is_none() {
        for fallback in &fallback_vs_paths {
            let p = PathBuf::from(fallback);
            if p.exists() {
                found_vs_path = Some(p);
                break;
            }
        }
    }

    if let Some(vs_path) = found_vs_path {
        let msvc_base = vs_path.join(r"VC\Tools\MSVC");
        if msvc_base.exists() {
            if let Ok(entries) = fs::read_dir(&msvc_base) {
                let mut versions: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_dir())
                    .collect();
                versions.sort();
                if let Some(latest_version) = versions.last() {
                    let candidates = [
                        latest_version.join(r"bin\Hostx64\x64\cl.exe"),
                        latest_version.join(r"bin\Hostx86\x64\cl.exe"),
                        latest_version.join(r"bin\Hostx64\x86\cl.exe"),
                    ];
                    for c in &candidates {
                        if c.exists() {
                            return Some(normalize_windows_path(c.to_str().unwrap()));
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn generate_c_cpp_properties(include_dirs: &[String], os: OS) -> String {
    let mut mac_framework_paths = Vec::new();
    if os == OS::Mac {
        let sdk_path = Path::new(MAC_SDK_ROOT);
        if sdk_path.exists() {
            mac_framework_paths.push(format!("{}/System/Library/Frameworks", MAC_SDK_ROOT));
        }
    }

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };

    let intellisense_mode = match os {
        OS::Windows => format!("windows-msvc-{}", arch),
        OS::Mac => format!("macos-clang-{}", arch),
        OS::Linux => format!("linux-gcc-{}", arch),
        OS::Unknown => format!("clang-{}", arch),
    };

    let defines = match os {
        OS::Windows => vec![
            "WIN32".to_string(),
            "_WIN32".to_string(),
            "TARGET_WIN32".to_string(),
            "_CRT_SECURE_NO_WARNINGS".to_string(),
            "_MSVC_LANG=201703L".to_string(),
            "_HAS_CXX17=1".to_string(),
            "OF_USING_STD_FS=1".to_string(),
            "OF_HAS_CPP17=1".to_string(),
            "__cpp_lib_filesystem=201703L".to_string(),
        ],
        OS::Mac => vec![
            "TARGET_OSX".to_string(),
            "OF_USING_STD_FS=1".to_string(),
            "OF_HAS_CPP17=1".to_string(),
            "__cpp_lib_filesystem=201703L".to_string(),
        ],
        OS::Linux => vec![
            "TARGET_LINUX".to_string(),
            "OF_USING_STD_FS=1".to_string(),
            "OF_HAS_CPP17=1".to_string(),
            "__cpp_lib_filesystem=201703L".to_string(),
        ],
        OS::Unknown => vec![],
    };

    let mut config_map = serde_json::Map::new();
    config_map.insert("name".to_string(), json!(os.to_str()));
    config_map.insert("includePath".to_string(), json!(include_dirs));
    config_map.insert("defines".to_string(), json!(defines));

    if os == OS::Windows {
        if let Some(compiler_path) = find_windows_compiler_path() {
            config_map.insert("compilerPath".to_string(), json!(compiler_path));
        }
    }

    config_map.insert("macFrameworkPath".to_string(), json!(mac_framework_paths));
    config_map.insert("cStandard".to_string(), json!("c11"));
    config_map.insert("cppStandard".to_string(), json!("c++17"));
    config_map.insert("intelliSenseMode".to_string(), json!(intellisense_mode));

    let config = json!({
        "configurations": [serde_json::Value::Object(config_map)],
        "version": 4
    });

    serde_json::to_string_pretty(&config).unwrap_or_default()
}

pub fn collect_windows_system_include_dirs() -> Vec<String> {
    let mut dirs = Vec::new();

    // 1. Discover MSVC include paths
    let vswhere_path = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let vs_install_path = if Path::new(vswhere_path).exists() {
        match process::Command::new(vswhere_path)
            .args(["-latest", "-property", "installationPath"])
            .output()
        {
            Ok(output) if output.status.success() => {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    Some(PathBuf::from(path_str))
                } else {
                    None
                }
            }
            _ => None,
        }
    } else {
        None
    };

    let fallback_vs_paths = [
        r"C:\Program Files\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Professional",
    ];

    let mut found_vs_path = vs_install_path;
    if found_vs_path.is_none() {
        for fallback in &fallback_vs_paths {
            let p = PathBuf::from(fallback);
            if p.exists() {
                found_vs_path = Some(p);
                break;
            }
        }
    }

    if let Some(vs_path) = found_vs_path {
        let msvc_base = vs_path.join(r"VC\Tools\MSVC");
        if msvc_base.exists() {
            if let Ok(entries) = fs::read_dir(&msvc_base) {
                let mut versions: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_dir())
                    .collect();
                versions.sort();
                if let Some(latest_version) = versions.last() {
                    let include_dir = latest_version.join("include");
                    if include_dir.exists() {
                        dirs.push(normalize_windows_path(include_dir.to_str().unwrap()));
                    }
                    let atlmfc_dir = latest_version.join(r"atlmfc\include");
                    if atlmfc_dir.exists() {
                        dirs.push(normalize_windows_path(atlmfc_dir.to_str().unwrap()));
                    }
                }
            }
        }
    }

    // 2. Discover Windows 10/11 SDK include paths
    let win_sdk_base = r"C:\Program Files (x86)\Windows Kits\10\Include";
    let sdk_path = Path::new(win_sdk_base);
    if sdk_path.exists() {
        if let Ok(entries) = fs::read_dir(sdk_path) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            if let Some(latest_version) = versions.last() {
                for sub in &["ucrt", "um", "shared", "winrt"] {
                    let sub_dir = latest_version.join(sub);
                    if sub_dir.exists() {
                        dirs.push(normalize_windows_path(sub_dir.to_str().unwrap()));
                    }
                }
            }
        }
    }

    dirs
}

pub fn dir_has_headers(dir: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if ext_str == "h" || ext_str == "hpp" || ext_str == "hxx" || ext_str == "hh" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn add_directories_recursively(
    dir: &Path,
    excludes: &[ExcludePattern],
    include_paths: &mut HashSet<String>,
) -> io::Result<()> {
    let norm = normalize_windows_path(dir.to_str().unwrap());
    if !is_excluded_dir(dir, excludes) {
        include_paths.insert(norm);
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

pub fn section_matches_os(section: &str, os: OS) -> bool {
    let s = section.to_lowercase();
    let s = s.trim();
    if s == "common" || s == "meta" {
        return true;
    }
    match os {
        OS::Windows => s.starts_with("vs") || s.starts_with("win") || s.starts_with("msys2"),
        OS::Mac => s.starts_with("osx") || s.starts_with("macos") || s.starts_with("darwin") || s.starts_with("ios"),
        OS::Linux => s.starts_with("linux"),
        OS::Unknown => true,
    }
}

pub fn parse_addon_includes(addon_path: &Path, os: OS) -> Vec<PathBuf> {
    let config_path = addon_path.join("addon_config.mk");
    let mut includes = Vec::new();

    if !config_path.exists() {
        return includes;
    }

    let file = match File::open(config_path) {
        Ok(f) => f,
        Err(_) => return includes,
    };
    let reader = BufReader::new(file);

    let mut current_section: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section header: e.g. linux:, osx:, vs:
        if line.ends_with(':') && !line.contains(' ') && !line.contains('=') {
            current_section = Some(line.trim_end_matches(':').to_string());
            continue;
        }

        if let Some(ref section) = current_section {
            if !section_matches_os(section, os) {
                continue;
            }
        }

        if line.starts_with("ADDON_INCLUDES") && !line.starts_with("ADDON_INCLUDES_EXCLUDE") {
            let parts: Vec<&str> = line.split(['=', '+']).collect();
            if parts.len() >= 2 {
                let raw_includes = parts.last().unwrap().trim();
                for inc in raw_includes.split_whitespace() {
                    let inc_clean = inc
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .replace('\\', "/");
                    if !inc_clean.is_empty() {
                        let inc_path = addon_path.join(&inc_clean);
                        if inc_path.exists() {
                            includes.push(inc_path);
                        }
                    }
                }
            }
        }
    }

    includes
}

pub fn parse_addon_excludes(addon_path: &Path, os: OS) -> Vec<ExcludePattern> {
    let config_path = addon_path.join("addon_config.mk");
    let mut excludes = Vec::new();

    if !config_path.exists() {
        return excludes;
    }

    let file = match File::open(config_path) {
        Ok(f) => f,
        Err(_) => return excludes,
    };
    let reader = BufReader::new(file);

    let mut current_section: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section header: e.g. linux:, osx:, vs:
        if line.ends_with(':') && !line.contains(' ') && !line.contains('=') {
            current_section = Some(line.trim_end_matches(':').to_string());
            continue;
        }

        // Check if section applies to current OS
        if let Some(ref section) = current_section {
            if !section_matches_os(section, os) {
                continue;
            }
        }

        if line.starts_with("ADDON_INCLUDES_EXCLUDE") {
            let parts: Vec<&str> = line.split(['=', '+']).collect();
            if parts.len() >= 2 {
                let pattern = parts.last().unwrap().trim();
                if !pattern.is_empty() {
                    let pattern_norm = pattern.replace('\\', "/");
                    let has_dir_wildcard = pattern_norm.ends_with("/%");
                    let has_wildcard = pattern_norm.ends_with('%');
                    let clean_pattern = pattern_norm.trim_end_matches("/%").trim_end_matches('%');
                    excludes.push(ExcludePattern {
                        pattern: addon_path.join(clean_pattern),
                        has_wildcard,
                        has_dir_wildcard,
                    });
                }
            }
        }
    }

    debug!(
        "excludes: {}",
        excludes
            .iter()
            .map(|e| e.pattern_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    excludes
}

pub fn is_excluded_dir(dir_abs: &Path, excludes: &[ExcludePattern]) -> bool {
    let dir_str = normalize_windows_path(dir_abs.to_str().unwrap());

    for exclude in excludes {
        let exclude_str = normalize_windows_path(exclude.pattern.to_str().unwrap());

        if exclude.has_dir_wildcard {
            if dir_str.starts_with(&exclude_str) {
                info!("excluded {} (dir wildcard {}/%)", dir_str, exclude_str);
                return true;
            }
        } else if exclude.has_wildcard {
            if dir_str.starts_with(&exclude_str) {
                info!("excluded {} (wildcard {}%)", dir_str, exclude_str);
                return true;
            }
        } else if dir_str == exclude_str {
            info!("excluded {} (exact match {})", dir_str, exclude_str);
            return true;
        }
    }

    debug!("not excluded {:?}", dir_abs);
    false
}

fn resolve_path(path: &Path) -> PathBuf {
    if path.exists() {
        let canonical = std::fs::canonicalize(path).unwrap();
        if cfg!(target_os = "windows") {
            PathBuf::from(normalize_windows_path(canonical.to_str().unwrap()))
        } else {
            canonical
        }
    } else {
        eprintln!("[Error] '{}' doesn't exist. Stops.", path.display());
        process::exit(1);
    }
}

pub fn normalize_windows_path(path_str: &str) -> String {
    let mut result = path_str.to_string();

    if result.starts_with(r"\\?\") {
        result = result[4..].to_string();
    }

    result = result.replace('\\', "/");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_windows_path() {
        assert_eq!(
            normalize_windows_path(r"C:\Users\foo\bar"),
            "C:/Users/foo/bar"
        );
        assert_eq!(
            normalize_windows_path(r"\\?\C:\Users\foo\bar"),
            "C:/Users/foo/bar"
        );
        assert_eq!(
            normalize_windows_path("/Users/foo/bar"),
            "/Users/foo/bar"
        );
    }

    #[test]
    fn test_os_to_str() {
        assert_eq!(OS::Mac.to_str(), "Mac");
        assert_eq!(OS::Linux.to_str(), "Linux");
        assert_eq!(OS::Windows.to_str(), "Win32");
        assert_eq!(OS::Unknown.to_str(), "Unknown");
    }

    #[test]
    fn test_generate_c_cpp_properties() {
        let include_dirs = vec![
            "C:/of/libs/openFrameworks".to_string(),
            "C:/my_project/src".to_string(),
        ];
        let json_str = generate_c_cpp_properties(&include_dirs, OS::Windows);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["version"], 4);
        assert_eq!(parsed["configurations"][0]["name"], "Win32");
        assert_eq!(parsed["configurations"][0]["cppStandard"], "c++17");
        assert_eq!(parsed["configurations"][0]["intelliSenseMode"], "windows-msvc-x64");
        assert!(parsed["configurations"][0]["defines"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("WIN32")));
    }

    #[test]
    fn test_exclude_pattern_matching() {
        let excludes = vec![
            ExcludePattern {
                pattern: PathBuf::from("C:/addon/libs/excluded_lib"),
                has_wildcard: false,
                has_dir_wildcard: true,
            },
            ExcludePattern {
                pattern: PathBuf::from("C:/addon/src/excluded_file.cpp"),
                has_wildcard: false,
                has_dir_wildcard: false,
            },
        ];

        assert!(is_excluded_dir(
            Path::new(r"C:\addon\libs\excluded_lib\sub"),
            &excludes
        ));
        assert!(is_excluded_dir(
            Path::new(r"C:\addon\src\excluded_file.cpp"),
            &excludes
        ));
        assert!(!is_excluded_dir(
            Path::new(r"C:\addon\libs\included_lib"),
            &excludes
        ));
    }

    #[test]
    fn test_section_matches_os() {
        assert!(section_matches_os("common", OS::Windows));
        assert!(section_matches_os("common", OS::Linux));
        assert!(section_matches_os("common", OS::Mac));

        assert!(section_matches_os("vs:", OS::Windows));
        assert!(section_matches_os("vs64", OS::Windows));
        assert!(section_matches_os("msys2", OS::Windows));
        assert!(!section_matches_os("linux", OS::Windows));
        assert!(!section_matches_os("osx", OS::Windows));
        assert!(!section_matches_os("android", OS::Windows));

        assert!(section_matches_os("osx", OS::Mac));
        assert!(section_matches_os("macos", OS::Mac));
        assert!(!section_matches_os("vs", OS::Mac));

        assert!(section_matches_os("linux", OS::Linux));
        assert!(section_matches_os("linux64", OS::Linux));
        assert!(!section_matches_os("vs", OS::Linux));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_collect_windows_system_include_dirs() {
        let dirs = collect_windows_system_include_dirs();
        if !dirs.is_empty() {
            assert!(dirs.iter().any(|d| d.contains("include") || d.contains("ucrt") || d.contains("um")));
        }
    }
}