use std::process::Command;

fn main() {
    if std::env::var("SPACEBOT_SKIP_FRONTEND_BUILD").is_ok() {
        ensure_dist_dir();
        return;
    }
    // Re-run if interface source files change
    println!("cargo:rerun-if-changed=interface/src/");
    println!("cargo:rerun-if-changed=interface/index.html");
    println!("cargo:rerun-if-changed=interface/package.json");
    println!("cargo:rerun-if-changed=interface/vite.config.ts");
    println!("cargo:rerun-if-changed=interface/tailwind.config.ts");

    let interface_dir = std::path::Path::new("interface");
    let allow_api_only_build = std::env::var("SPACEBOT_ALLOW_API_ONLY_BUILD").is_ok();

    // By default, local builds are expected to include the embedded dashboard.
    // API-only builds must opt out explicitly.
    if !interface_dir.join("node_modules").exists() {
        if allow_api_only_build {
            eprintln!(
                "cargo:warning=interface/node_modules not found; building API-only binary because SPACEBOT_ALLOW_API_ONLY_BUILD is set"
            );
            ensure_dist_dir();
            return;
        }
        panic!(
            "interface/node_modules not found. Run `cd interface && bun install`, or set SPACEBOT_ALLOW_API_ONLY_BUILD=1 to explicitly build without the dashboard."
        );
    }

    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(interface_dir)
        .status();

    match status {
        Ok(s) if s.success() => {
            if !interface_dir.join("dist/index.html").exists() {
                if allow_api_only_build {
                    eprintln!(
                        "cargo:warning=frontend build finished without dist/index.html; continuing because SPACEBOT_ALLOW_API_ONLY_BUILD is set"
                    );
                    ensure_dist_dir();
                    return;
                }
                panic!(
                    "frontend build completed but interface/dist/index.html is missing. Set SPACEBOT_ALLOW_API_ONLY_BUILD=1 only if you intentionally want an API-only binary."
                );
            }
        }
        Ok(s) => {
            if allow_api_only_build {
                eprintln!(
                    "cargo:warning=frontend build exited with {s}; continuing with API-only binary because SPACEBOT_ALLOW_API_ONLY_BUILD is set"
                );
                ensure_dist_dir();
                return;
            }
            panic!(
                "frontend build failed with {s}. Fix the interface build, or set SPACEBOT_ALLOW_API_ONLY_BUILD=1 to explicitly build without the dashboard."
            );
        }
        Err(e) => {
            if allow_api_only_build {
                eprintln!(
                    "cargo:warning=failed to run `bun run build`: {e}. Continuing with API-only binary because SPACEBOT_ALLOW_API_ONLY_BUILD is set."
                );
                ensure_dist_dir();
                return;
            }
            panic!(
                "failed to run `bun run build`: {e}. Install bun, or set SPACEBOT_ALLOW_API_ONLY_BUILD=1 to explicitly build without the dashboard."
            );
        }
    }
}

/// rust-embed requires the folder to exist even if empty.
fn ensure_dist_dir() {
    let dist = std::path::Path::new("interface/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).ok();
    }
}
