use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=Info.plist.in");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must provide CARGO_MANIFEST_DIR"),
    );
    let template_path = manifest_dir.join("Info.plist.in");
    let template = fs::read_to_string(&template_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", template_path.display()));
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo must provide CARGO_PKG_VERSION");
    let plist = template.replace("@EDGEMOUSE_VERSION@", &version);

    let output_path = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"))
        .join("EdgeMouse-Info.plist");
    fs::write(&output_path, plist)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output_path.display()));

    println!(
        "cargo:rustc-link-arg-bin=edgemouse=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        output_path.display()
    );
}
