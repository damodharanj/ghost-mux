use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned ghostty commit. Update this to pull a newer version.
const GHOSTTY_REPO: &str = "https://github.com/ghostty-org/ghostty.git";
const GHOSTTY_COMMIT: &str = "bebca84668947bfc92b9a30ed58712e1c34eee1d";

fn main() {
    // docs.rs has no Zig toolchain. The checked-in bindings in src/bindings.rs
    // are enough for generating documentation, so skip the entire native
    // build when running under docs.rs.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_NO_VENDOR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=TARGET");
    println!("cargo:rerun-if-env-changed=HOST");
    println!("cargo:rerun-if-env-changed=ZIG");
    println!("cargo:rerun-if-changed=crates/libghostty-vt-sys/build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let target = env::var("TARGET").expect("TARGET must be set");
    let host = env::var("HOST").expect("HOST must be set");

    // Locate ghostty source: env override > fetch into OUT_DIR.
    let ghostty_dir = match env::var("GHOSTTY_SOURCE_DIR") {
        Ok(dir) => {
            let p = PathBuf::from(dir);
            assert!(
                p.join("build.zig").exists(),
                "GHOSTTY_SOURCE_DIR does not contain build.zig: {}",
                p.display()
            );
            p
        }
        Err(_) => fetch_ghostty(&out_dir),
    };

    // Clean .zig-cache to prevent stale cache or compiler-rt version mismatch issues
    // across different GitHub Actions runner OS/SDK versions.
    let zig_cache = ghostty_dir.join(".zig-cache");
    if zig_cache.exists() {
        let _ = std::fs::remove_dir_all(&zig_cache);
    }

    // Build libghostty-vt via zig.
    let install_prefix = out_dir.join("ghostty-install");

    let zig = resolve_zig_executable();
    let mut build = Command::new(&zig);
    let local_cache = ghostty_dir.join(".zig-cache");
    build
        .arg("build")
        .arg("-Demit-lib-vt")
        .arg("--prefix")
        .arg(&install_prefix)
        .arg("--cache-dir")
        .arg(&local_cache)
        .arg("--global-cache-dir")
        .arg(&local_cache)
        .current_dir(&ghostty_dir);
    let opt_level = env::var("OPT_LEVEL").unwrap_or_else(|_| "0".to_string());
    let zig_optimize = match opt_level.as_str() {
        "0" => "Debug",
        "s" | "z" => "ReleaseSmall",
        _ => "ReleaseFast",
    };
    build.arg(format!("-Doptimize={zig_optimize}"));

    // On macOS, ensure SDKROOT, DEVELOPER_DIR, and MACOSX_DEPLOYMENT_TARGET are set so Zig can link against libSystem.
    if target.contains("darwin") {
        let sdk = env::var("SDKROOT").ok().filter(|s| !s.is_empty()).or_else(|| {
            Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        if let Some(sdk_path) = sdk {
            build.env("SDKROOT", &sdk_path);
        }

        let dev_dir = env::var("DEVELOPER_DIR").ok().filter(|s| !s.is_empty()).or_else(|| {
            Command::new("xcode-select")
                .arg("-p")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        if let Some(dev_path) = dev_dir {
            build.env("DEVELOPER_DIR", &dev_path);
        }

        let deploy_target = env::var("MACOSX_DEPLOYMENT_TARGET")
            .unwrap_or_else(|_| "11.0".to_string());
        build.env("MACOSX_DEPLOYMENT_TARGET", deploy_target);
    }

    // Only pass -Dtarget when cross-compiling. For native builds, let zig
    // auto-detect the host (matches how ghostty's own CMakeLists.txt works).
    if target != host {
        let zig_target = zig_target(&target);
        build.arg(format!("-Dtarget={zig_target}"));
    }

    let zig_context = format!("zig build (using {})", zig.display());
    run_with_retry(&mut build, &zig_context, 3);

    let lib_dir = install_prefix.join("lib");
    let include_dir = install_prefix.join("include");

    let is_msvc = target.contains("msvc");
    let static_lib_name = if is_msvc {
        "ghostty-vt.lib"
    } else {
        "libghostty-vt.a"
    };

    assert!(
        lib_dir.join(static_lib_name).exists(),
        "expected static library at {}",
        lib_dir.join(static_lib_name).display()
    );
    assert!(
        include_dir.join("ghostty").join("vt.h").exists(),
        "expected header at {}",
        include_dir.join("ghostty").join("vt.h").display()
    );

    // Link the main ghostty-vt static archive.
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");

    // The zig build produces dependency archives (highway, simdutf, utfcpp).
    // Search both local and global Zig caches and emit link-search directives.
    let dep_libs = if is_msvc {
        vec!["highway.lib", "simdutf.lib", "utfcpp.lib"]
    } else {
        vec!["libhighway.a", "libsimdutf.a", "libutfcpp.a"]
    };

    let mut search_roots = vec![local_cache];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        search_roots.push(home.join(".cache").join("zig"));
        search_roots.push(home.join("Library").join("Caches").join("org.ziglang.zig"));
    }

    let mut found_libs = std::collections::HashSet::new();
    for root in search_roots {
        if root.is_dir() {
            find_and_link_dep_libs(&root, &dep_libs, is_msvc, &mut found_libs, &lib_dir, 0);
        }
    }

    // Zig-compiled static libraries on macOS and Linux need the C++ runtime.
    if target.contains("darwin") {
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=c++");
    } else if target.contains("linux") {
        // Zig compiles C++ sources using LLVM libc++ (std::__1), so link c++ and c++abi
        // along with standard system libraries.
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=c++abi");
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=dl");
    }
    println!("cargo:include={}", include_dir.display());
}

/// Clone ghostty at the pinned commit into OUT_DIR/ghostty-src.
/// Reuses an existing clone if the commit matches.
fn fetch_ghostty(out_dir: &Path) -> PathBuf {
    let src_dir = out_dir.join("ghostty-src");
    let stamp = src_dir.join(".ghostty-commit");

    // Skip fetch if we already have the right commit.
    if stamp.exists()
        && let Ok(existing) = std::fs::read_to_string(&stamp)
            && existing.trim() == GHOSTTY_COMMIT {
                return src_dir;
            }

    // Clean and clone fresh.
    if src_dir.exists() {
        let _ = std::fs::remove_dir_all(&src_dir);
    }

    eprintln!("Fetching ghostty {GHOSTTY_COMMIT} ...");

    let mut clone = Command::new("git");
    clone
        .arg("clone")
        .arg("--filter=blob:none")
        .arg("--no-checkout")
        .arg(GHOSTTY_REPO)
        .arg(&src_dir);
    run_with_retry(&mut clone, "git clone ghostty", 3);

    let mut checkout = Command::new("git");
    checkout
        .arg("checkout")
        .arg(GHOSTTY_COMMIT)
        .current_dir(&src_dir);
    run_with_retry(&mut checkout, "git checkout ghostty commit", 3);

    std::fs::write(&stamp, GHOSTTY_COMMIT).unwrap_or_else(|e| panic!("failed to write stamp: {e}"));

    src_dir
}

fn run_with_retry(command: &mut Command, context: &str, retries: usize) {
    let mut last_err = String::new();
    for attempt in 1..=retries {
        let output = match command.output() {
            Ok(out) => out,
            Err(e) => {
                last_err = format!("failed to spawn {context}: {e}");
                if attempt < retries {
                    eprintln!("Retrying {context} (attempt {attempt}/{retries}) after spawn error: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                }
                panic!("failed to execute {context}: {e}");
            }
        };

        if output.status.success() {
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        last_err = format!(
            "{context} failed with {}.\n--- STDOUT ---\n{stdout}\n--- STDERR ---\n{stderr}",
            output.status
        );

        if attempt < retries {
            eprintln!(
                "Retrying {context} (attempt {attempt}/{retries}) after failure:\n{stderr}"
            );
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    panic!("{last_err}");
}

fn resolve_zig_executable() -> PathBuf {
    if let Ok(path) = env::var("ZIG") {
        return PathBuf::from(path);
    }

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set for build script"),
    );
    let exe = if cfg!(windows) { "zig.exe" } else { "zig" };

    for ancestor in manifest_dir.ancestors() {
        let candidate = ancestor
            .join(".tools")
            .join("zig")
            .join("toolchain")
            .join(exe);
        if candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from("zig")
}

fn zig_target(target: &str) -> String {
    let value = match target {
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gnu",
        "x86_64-unknown-linux-musl" => "x86_64-linux-musl",
        "aarch64-unknown-linux-gnu" => "aarch64-linux-gnu",
        "aarch64-unknown-linux-musl" => "aarch64-linux-musl",
        "aarch64-apple-darwin" => "aarch64-macos-none",
        "x86_64-apple-darwin" => "x86_64-macos-none",
        "x86_64-pc-windows-msvc" => "x86_64-windows-msvc",
        "aarch64-pc-windows-msvc" => "aarch64-windows-msvc",
        other => panic!("unsupported Rust target for vendored build: {other}"),
    };
    value.to_owned()
}

fn find_and_link_dep_libs(
    dir: &Path,
    dep_libs: &[&str],
    is_msvc: bool,
    found: &mut std::collections::HashSet<String>,
    dest_dir: &Path,
    depth: usize,
) {
    if depth > 15 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            for lib in dep_libs {
                let src_file = path.join(lib);
                if src_file.is_file() {
                    let lib_name = if is_msvc {
                        lib.trim_end_matches(".lib")
                    } else {
                        lib.trim_start_matches("lib").trim_end_matches(".a")
                    };
                    if found.insert(lib_name.to_string()) {
                        let dest_file = dest_dir.join(lib);
                        let _ = std::fs::copy(&src_file, &dest_file);
                        println!("cargo:rustc-link-search=native={}", path.display());
                        println!("cargo:rustc-link-lib=static={lib_name}");
                    }
                }
            }
            find_and_link_dep_libs(&path, dep_libs, is_msvc, found, dest_dir, depth + 1);
        }
    }
}
