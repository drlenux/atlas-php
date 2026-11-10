use std::{env, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=ATLAS_LIBPQ_DIR");
    if let Ok(directory) = env::var("ATLAS_LIBPQ_DIR") {
        println!("cargo:rustc-link-search=native={directory}");
        return;
    }
    for program in [
        "pg_config",
        "/opt/homebrew/opt/postgresql@17/bin/pg_config",
        "/opt/homebrew/opt/libpq/bin/pg_config",
    ] {
        if let Ok(output) = Command::new(program).arg("--libdir").output() {
            if output.status.success() {
                let directory = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if Path::new(&directory).is_dir() {
                    println!("cargo:rustc-link-search=native={directory}");
                    break;
                }
            }
        }
    }
}
