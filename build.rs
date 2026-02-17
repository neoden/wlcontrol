use std::{env, fs, path::PathBuf, process::Command};

fn check_program(name: &str, install_hint: &str) {
    match Command::new(name).arg("--version").output() {
        Ok(output) if output.status.success() => {}
        _ => {
            eprintln!("error: `{name}` not found. Install it:");
            eprintln!("  {install_hint}");
            std::process::exit(1);
        }
    }
}

fn main() {
    check_program(
        "blueprint-compiler",
        "sudo apt install blueprint-compiler  # Ubuntu/Debian",
    );
    check_program(
        "pkg-config",
        "sudo apt install pkg-config  # Ubuntu/Debian",
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ui_out = out_dir.join("ui");
    fs::create_dir_all(&ui_out).unwrap();

    // Compile .blp → .ui into OUT_DIR
    let blp_dir = PathBuf::from("data/resources/ui");
    for entry in fs::read_dir(&blp_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "blp") {
            let stem = path.file_stem().unwrap();
            let ui_file = ui_out.join(stem).with_extension("ui");

            let status = Command::new("blueprint-compiler")
                .args(["compile", "--output"])
                .arg(&ui_file)
                .arg(&path)
                .status()
                .expect("failed to run blueprint-compiler");

            assert!(status.success(), "blueprint-compiler failed for {:?}", path);
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    println!("cargo:rerun-if-changed=data/resources/resources.gresource.xml");
    println!("cargo:rerun-if-changed=data/resources/style.css");

    // Compile gresource — look for .ui in OUT_DIR, other assets in data/resources
    glib_build_tools::compile_resources(
        &[out_dir.to_str().unwrap(), "data/resources"],
        "data/resources/resources.gresource.xml",
        "wlcontrol.gresource",
    );
}
