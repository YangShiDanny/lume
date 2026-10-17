use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/lume.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon = manifest_dir.join("assets").join("lume.ico");
    if !icon.is_file() {
        panic!("Windows icon resource is missing: {}", icon.display());
    }

    let staged_icon = out_dir.join("lume.ico");
    let resource_file = out_dir.join("lume.rc");
    let compiled_resource = out_dir.join("lume.res");
    fs::copy(&icon, &staged_icon).expect("failed to stage lume.ico");
    fs::write(&resource_file, "1 ICON \"lume.ico\"\r\n").expect("failed to write lume.rc");

    let rc = find_resource_compiler();
    let status = Command::new(&rc)
        .current_dir(&out_dir)
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled_resource)
        .arg(&resource_file)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {}", rc.display(), error));
    if !status.success() {
        panic!("resource compiler failed with {}", status);
    }

    println!("cargo:rustc-link-arg={}", compiled_resource.display());
}

fn find_resource_compiler() -> PathBuf {
    if let Some(rc) = env::var_os("RC").map(PathBuf::from) {
        if rc.is_file() || rc.components().count() == 1 {
            return rc;
        }
    }

    if let Ok(output) = Command::new("where.exe").arg("rc.exe").output() {
        if output.status.success() {
            if let Some(path) = String::from_utf8_lossy(&output.stdout).lines().next() {
                let path = PathBuf::from(path.trim());
                if path.is_file() {
                    return path;
                }
            }
        }
    }

    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let mut roots = Vec::new();
    if let Some(sdk_dir) = env::var_os("WindowsSdkDir") {
        roots.push(PathBuf::from(sdk_dir).join("bin"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        roots.push(
            PathBuf::from(program_files)
                .join("Windows Kits")
                .join("10")
                .join("bin"),
        );
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        roots.push(
            PathBuf::from(program_files)
                .join("Windows Kits")
                .join("10")
                .join("bin"),
        );
    }

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut versions: Vec<PathBuf> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort_by(|a, b| b.cmp(a));
        for version in versions {
            let candidate = version.join(arch).join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    panic!("rc.exe was not found; install the Windows SDK or set RC to its path");
}
