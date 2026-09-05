#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "macos")]
    if let Some(exit_code) = run_embedded_macos_agent() {
        std::process::exit(exit_code);
    }
    edgemouse_desktop::run();
}

#[cfg(target_os = "macos")]
fn run_embedded_macos_agent() -> Option<i32> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--agent-run")) {
        return None;
    }
    let Some(config_path) = arguments.next() else {
        eprintln!("edgemouse: --agent-run requires a configuration path");
        return Some(2);
    };
    if arguments.next().is_some() {
        eprintln!("edgemouse: --agent-run accepts exactly one configuration path");
        return Some(2);
    }
    match edgemouse_agent::runtime::run(std::path::Path::new(&config_path)) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("edgemouse: {error}");
            Some(1)
        }
    }
}
