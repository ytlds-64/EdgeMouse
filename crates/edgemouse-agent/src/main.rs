mod config;
mod control;
mod discovery;
mod network;
mod pairing;
mod platform;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod runtime;

use edgemouse_core::{
    Edge, Effect, NodeId, PhysicalMouseEvent, Point, Rect, Screen, ScreenId, Session,
    SessionConfig, Topology, Vector,
};
use edgemouse_protocol::{WireMessage, encode_frame};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("edgemouse: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("doctor") => doctor(),
        Some("demo") => demo(),
        Some("identity") => {
            let directory = arguments
                .next()
                .ok_or("identity requires an output directory")?;
            ensure_no_extra_arguments(arguments)?;
            generate_identity(Path::new(&directory))
        }
        Some("check-config") => {
            let config = arguments
                .next()
                .ok_or("check-config requires a TOML config path")?;
            ensure_no_extra_arguments(arguments)?;
            check_config(Path::new(&config))
        }
        Some("discover") => {
            let config = arguments
                .next()
                .ok_or("discover requires a TOML config path")?;
            ensure_no_extra_arguments(arguments)?;
            discover_peer(Path::new(&config))
        }
        Some("pair") => pair(arguments),
        Some("run") => {
            let config = arguments.next().ok_or("run requires a TOML config path")?;
            ensure_no_extra_arguments(arguments)?;
            run_agent(Path::new(&config))
        }
        Some("status") => {
            ensure_no_extra_arguments(arguments)?;
            status()
        }
        Some("stop") => {
            ensure_no_extra_arguments(arguments)?;
            stop()
        }
        Some("version" | "--version" | "-V") => {
            println!("edgemouse {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("help" | "--help" | "-h") | None => {
            usage();
            Ok(())
        }
        Some(command) => Err(format!("unknown command `{command}`; run `edgemouse help`").into()),
    }
}

fn usage() {
    println!(
        "EdgeMouse MVP\n\nUSAGE:\n    edgemouse <COMMAND>\n\nCOMMANDS:\n    doctor                                Check platform APIs and permissions\n    identity <DIRECTORY>                  Generate this node's certificate and private key\n    pair host <CONFIG>                    Show a one-time code and offer secure pairing\n    pair join <CONFIG> <CODE> [HOST]      Pair by discovery or a direct host IP\n    check-config <CONFIG>                 Validate configuration and certificate pairing\n    discover <CONFIG>                     Find the configured trusted peer on the LAN\n    run <CONFIG>                          Connect to the trusted peer and enable edge switching\n    status                                Show whether the local agent is running\n    stop                                  Safely stop the local agent\n    demo                                  Simulate a Windows-to-macOS edge transition\n    version                               Print the build version\n    help                                  Show this help"
    );
}

fn status() -> Result<(), Box<dyn Error>> {
    match control::query_status()? {
        Some(status) => {
            println!("EdgeMouse is running");
            println!("Version : {}", status.version);
            println!("Process : {}", status.process_id);
        }
        None => println!("EdgeMouse is not running"),
    }
    Ok(())
}

fn stop() -> Result<(), Box<dyn Error>> {
    let Some(status) = control::request_stop()? else {
        println!("EdgeMouse is not running");
        return Ok(());
    };
    println!(
        "Stopping EdgeMouse {} (process {})…",
        status.version, status.process_id
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if control::query_status()?.is_none() {
            println!("EdgeMouse stopped safely");
            return Ok(());
        }
    }
    println!("Stop requested; shutdown is still in progress");
    Ok(())
}

fn pair(mut arguments: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let mode = arguments
        .next()
        .ok_or("pair requires `host <CONFIG>` or `join <CONFIG> <CODE>`")?;
    let config_path = arguments.next().ok_or("pair requires a TOML config path")?;
    let config = config::PairingConfig::load(Path::new(&config_path))?;
    let stopping = Arc::new(AtomicBool::new(false));
    let handler_state = Arc::clone(&stopping);
    ctrlc::set_handler(move || handler_state.store(true, Ordering::Release))?;

    let result = match mode.as_str() {
        "host" => {
            ensure_no_extra_arguments(arguments)?;
            let host = pairing::PairingHost::start(config)?;
            println!("Secure pairing is ready");
            println!("One-time code: {}", host.formatted_code());
            println!(
                "On the other computer run: edgemouse pair join <CONFIG> {}",
                host.formatted_code()
            );
            println!(
                "If broadcast discovery is blocked, append this computer's IP address to that command."
            );
            println!("The code expires in 5 minutes and allows at most 3 attempts.");
            println!("Press Ctrl+C to cancel.");
            host.run(&stopping)?
        }
        "join" => {
            let code = arguments
                .next()
                .ok_or("pair join requires the 8-digit code shown by the host")?;
            let host = arguments.next();
            ensure_no_extra_arguments(arguments)?;
            if let Some(host) = &host {
                println!("Connecting directly to pairing host {host}…");
            } else {
                println!(
                    "Looking for a pairing host on UDP {}…",
                    discovery::DISCOVERY_PORT
                );
            }
            pairing::join(config, &code, host.as_deref(), &stopping)?
        }
        _ => return Err(format!("unknown pair mode `{mode}`; use `host` or `join`").into()),
    };

    println!("Secure pairing completed");
    println!("Peer name   : {}", result.peer_name);
    println!(
        "Peer node   : {}",
        edgemouse_transport::format_node_id(result.peer_node)
    );
    println!("Certificate : {}", result.certificate_path.display());
    if result.certificate_installed {
        println!("Trust status: new certificate saved");
    } else {
        println!("Trust status: existing identical certificate kept");
    }
    println!("Next step   : run `edgemouse check-config {config_path}` on this computer");
    Ok(())
}

fn check_config(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = config::LoadedConfig::load(path)?;
    println!("Configuration is valid");
    println!(
        "Local node : {}",
        edgemouse_transport::format_node_id(config.local_node)
    );
    println!(
        "Peer node  : {}",
        edgemouse_transport::format_node_id(config.peer_node)
    );
    println!("Listen     : {}", config.transport.bind_address);
    match config.peer_address {
        config::PeerAddress::Static(address) => println!("Peer       : {address}"),
        config::PeerAddress::Auto => {
            println!("Peer       : auto (UDP {})", discovery::DISCOVERY_PORT)
        }
    }
    println!("Local screen: {}", config.local_screen.0);
    Ok(())
}

fn discover_peer(path: &Path) -> Result<(), Box<dyn Error>> {
    let config = config::LoadedConfig::load(path)?;
    let stopping = Arc::new(AtomicBool::new(false));
    let handler_state = Arc::clone(&stopping);
    ctrlc::set_handler(move || handler_state.store(true, Ordering::Release))?;
    println!(
        "Looking for trusted peer {} on UDP {}…",
        edgemouse_transport::format_node_id(config.peer_node),
        discovery::DISCOVERY_PORT
    );
    let request = discovery::DiscoveryRequest {
        local_node: config.local_node,
        expected_peer: config.peer_node,
        local_name: config.transport.local_name,
        quic_port: config.transport.bind_address.port(),
        timeout: config.transport.connect_timeout,
    };
    match discovery::discover_trusted_peer(&request, &stopping) {
        Ok(peer) => {
            println!("Trusted peer found: {} at {}", peer.name, peer.address);
            Ok(())
        }
        Err(_) if stopping.load(Ordering::Acquire) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn generate_identity(directory: &Path) -> Result<(), Box<dyn Error>> {
    let generated = edgemouse_transport::Identity::generate()?;
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "failed to create identity directory {}: {error}",
            directory.display()
        )
    })?;
    let certificate = directory.join("certificate.der");
    let private_key = directory.join("private-key.der");
    refuse_overwrite(&certificate)?;
    refuse_overwrite(&private_key)?;
    fs::write(&certificate, generated.certificate)?;
    if let Err(error) = write_private_key(&private_key, &generated.private_key) {
        drop(fs::remove_file(&certificate));
        return Err(error);
    }
    println!(
        "Node ID    : {}",
        edgemouse_transport::format_node_id(generated.node_id)
    );
    println!("Certificate: {}", certificate.display());
    println!("Private key: {}", private_key.display());
    println!("Share only certificate.der with the other machine.");
    Ok(())
}

fn write_private_key(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn refuse_overwrite(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        Err(format!(
            "refusing to overwrite existing identity file {}",
            path.display()
        )
        .into())
    } else {
        Ok(())
    }
}

fn ensure_no_extra_arguments(
    arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let extra: Vec<_> = arguments.collect();
    if extra.is_empty() {
        Ok(())
    } else {
        Err(format!("unexpected arguments: {}", extra.join(" ")).into())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn run_agent(path: &Path) -> Result<(), Box<dyn Error>> {
    runtime::run(path)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn run_agent(_path: &Path) -> Result<(), Box<dyn Error>> {
    Err("live input control is supported only on Windows and macOS".into())
}

fn doctor() -> Result<(), Box<dyn Error>> {
    let status = platform::current_status();
    println!("Operating system : {}", status.operating_system);
    println!("Capture API      : {}", status.capture_api);
    println!("Injection API    : {}", status.injection_api);
    match status.permission_granted {
        Some(true) => println!("Input permission : granted"),
        Some(false) => {
            println!("Input permission : not granted");
            println!(
                "Action required  : enable EdgeMouse under Privacy & Security > Accessibility"
            );
        }
        None => println!("Input permission : checked when the native adapter starts"),
    }
    Ok(())
}

fn demo() -> Result<(), Box<dyn Error>> {
    const WINDOWS: NodeId = NodeId(1);
    const MACOS: NodeId = NodeId(2);
    const WINDOWS_SCREEN: ScreenId = ScreenId(1);
    const MACOS_SCREEN: ScreenId = ScreenId(2);

    let mut topology = Topology::default();
    topology.add_screen(Screen::new(
        WINDOWS_SCREEN,
        WINDOWS,
        "Windows display",
        Rect::new(Point::new(0.0, 0.0), 1920.0, 1080.0)?,
        1.0,
    )?)?;
    topology.add_screen(Screen::new(
        MACOS_SCREEN,
        MACOS,
        "Mac display",
        Rect::new(Point::new(0.0, 0.0), 1512.0, 982.0)?,
        2.0,
    )?)?;
    topology.connect_bidirectional(WINDOWS_SCREEN, Edge::Right, MACOS_SCREEN)?;

    let mut session = Session::new(
        WINDOWS,
        topology,
        WINDOWS_SCREEN,
        Point::new(1918.0, 540.0),
        SessionConfig::default(),
    )?;
    println!(
        "Initial state: {:?} at {:?}",
        session.state(),
        session.pointer()
    );

    for (now_ms, movement) in [
        (100, Vector::new(8.0, 0.0)),
        (110, Vector::new(20.0, 4.0)),
        (120, Vector::new(-40.0, 0.0)),
    ] {
        let result = session.handle_input(PhysicalMouseEvent::Move { movement }, now_ms)?;
        println!(
            "move {movement:?} => {:?}, state={:?}, pointer={:?}",
            result.disposition,
            session.state(),
            session.pointer()
        );
        for effect in result.effects {
            print_effect(effect)?;
        }
    }
    Ok(())
}

fn print_effect(effect: Effect) -> Result<(), Box<dyn Error>> {
    match effect {
        Effect::Send { peer, event } => {
            let frame = encode_frame(&WireMessage::Mouse {
                session_id: 1,
                event,
            })?;
            println!(
                "  network => peer={}, sequence={}, encoded={} bytes, event={:?}",
                peer.0,
                event.sequence,
                frame.len(),
                event.event
            );
        }
        Effect::SendKeyboard { peer, event } => {
            let frame = encode_frame(&WireMessage::Keyboard {
                session_id: 1,
                event,
            })?;
            println!(
                "  keyboard => peer={}, sequence={}, encoded={} bytes, event={:?}",
                peer.0,
                event.sequence,
                frame.len(),
                event.event
            );
        }
        other => println!("  platform => {other:?}"),
    }
    Ok(())
}
