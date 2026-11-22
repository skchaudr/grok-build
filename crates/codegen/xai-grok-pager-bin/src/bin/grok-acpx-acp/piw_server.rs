//! Starting the pi-workflows server on demand, exactly the way Pi's own
//! client does it: its viewer CLI's `server start` command runs
//! `ensureAvailable` (pi-workflows `src/client/client.ts`), which starts the
//! package's detached server entry only when no server answers on the
//! socket, waits for its ready handshake, and never starts a second server
//! beside a live one (the server's own lock fences that).
//!
//! The driver calls [`socket_presence`] before anything that needs the
//! server, and starts one only when the socket is missing or refuses the
//! connection. Package resolution follows Pi's `packages` list in the Pi
//! agent directory's `settings.json`: an entry that is a path to a
//! pi-workflows checkout, or `npm:@osolmaz/pi-workflows` installed under the
//! agent directory's `npm/node_modules`.

use serde_json::Value;
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use xai_grok_pager::piw::client::PiwClient;

/// How long `pi-workflows server start` gets. The CLI itself stops after
/// 10 s (`START_TIMEOUT_MS`), so this only bounds a hung child.
pub const SERVER_START_TIMEOUT: Duration = Duration::from_secs(15);

/// The pi-workflows package name, the marker that a directory really is one.
const PIWORKFLOWS_PACKAGE_NAME: &str = "@osolmaz/pi-workflows";

/// What answers (or does not) on the machine's default pi-workflows socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPresence {
    /// Something accepted a connection: a server is running.
    Live,
    /// The socket file does not exist.
    Missing,
    /// The socket file exists but refuses the connection.
    Refused,
    /// The socket file exists but the connect failed some other way.
    /// Starting a server cannot fix this, so no start is attempted.
    Other,
}

impl SocketPresence {
    /// Whether a server should be started in this state.
    pub fn needs_start(self) -> bool {
        matches!(self, SocketPresence::Missing | SocketPresence::Refused)
    }
}

/// Probe the machine's default pi-workflows socket. Never starts anything.
/// `None` when no default socket path can be computed at all.
pub fn socket_presence() -> Option<SocketPresence> {
    PiwClient::default_socket_path()
        .as_deref()
        .map(socket_presence_at)
}

/// The same probe for an explicit socket path.
pub fn socket_presence_at(socket: &Path) -> SocketPresence {
    if !socket.exists() {
        return SocketPresence::Missing;
    }
    match UnixStream::connect(socket) {
        Ok(_) => SocketPresence::Live,
        Err(err) if err.kind() == ErrorKind::ConnectionRefused => SocketPresence::Refused,
        Err(_) => SocketPresence::Other,
    }
}

/// Why package resolution failed: every entry that looked like a
/// pi-workflows package and the path that was tried for it, so the
/// transcript can name them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageResolution {
    /// `<pi agent dir>/settings.json`, the list that was read.
    pub settings_path: PathBuf,
    /// One line per candidate: the entry from the list and the path tried.
    pub tried: Vec<String>,
}

/// Resolve the pi-workflows package the way Pi does: read `packages` from
/// `<agent_dir>/settings.json` in order; a string entry (or an object
/// entry's `source`) is either a path to a checkout or
/// `npm:@osolmaz/pi-workflows`, which lives under
/// `<agent_dir>/npm/node_modules/@osolmaz/pi-workflows`. The first entry
/// whose directory really is a pi-workflows package (`package.json` name
/// `@osolmaz/pi-workflows`) wins.
pub fn resolve_piworkflows_package(
    agent_dir: &Path,
) -> std::result::Result<PathBuf, PackageResolution> {
    let settings_path = agent_dir.join("settings.json");
    let mut resolution = PackageResolution {
        settings_path: settings_path.clone(),
        tried: Vec::new(),
    };
    let settings: Option<Value> = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let Some(settings) = settings else {
        return Err(resolution);
    };
    let Some(entries) = settings.get("packages").and_then(Value::as_array) else {
        return Err(resolution);
    };
    for entry in entries {
        // A package entry is either a plain string or an object with a
        // `source` string (the same shapes Pi's settings.json uses).
        let Some(source) = entry_source(entry) else { continue };
        let Some(candidate) = entry_candidate(&source, agent_dir) else { continue };
        resolution.tried.push(candidate.display().to_string());
        if is_piworkflows_package(&candidate) {
            return Ok(candidate);
        }
    }
    Err(resolution)
}

/// The Pi agent directory: `$PI_AGENT_DIR`, else `$HOME/.pi/agent`.
pub fn pi_agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_AGENT_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(Path::new(&home).join(".pi").join("agent"))
}

/// The `source` of one `packages` entry: the string itself, or the object's
/// `source` field.
fn entry_source(entry: &Value) -> Option<String> {
    match entry {
        Value::String(source) => Some(source.clone()),
        Value::Object(map) => match map.get("source") {
            Some(Value::String(source)) => Some(source.clone()),
            _ => None,
        },
        _ => None,
    }
    .filter(|source| !source.is_empty())
}

/// The package directory one entry names, if the entry can name one:
/// `npm:@osolmaz/pi-workflows` under the agent directory's
/// `npm/node_modules`, or a filesystem path.
fn entry_candidate(source: &str, agent_dir: &Path) -> Option<PathBuf> {
    if source == "npm:@osolmaz/pi-workflows" {
        return Some(
            agent_dir
                .join("npm")
                .join("node_modules")
                .join("@osolmaz")
                .join("pi-workflows"),
        );
    }
    if source.starts_with("npm:") {
        return None;
    }
    Some(PathBuf::from(source))
}

/// Is this directory a pi-workflows package? Its `package.json` must carry
/// the package name.
fn is_piworkflows_package(dir: &Path) -> bool {
    let Some(text) = std::fs::read_to_string(dir.join("package.json")).ok() else {
        return false;
    };
    let Some(value) = serde_json::from_str::<Value>(&text).ok() else {
        return false;
    };
    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name == PIWORKFLOWS_PACKAGE_NAME)
}

/// The command that starts the server, chosen the way the package chooses
/// its own CLI: the built viewer CLI when `dist` exists, the same CLI from
/// source through tsx when it does not (the choice `startDetached` makes
/// between the built and the source server entry). Both wait for the
/// server's ready handshake before exiting, and start nothing when a server
/// already answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStartCommand {
    pub program: PathBuf,
    pub args: Vec<PathBuf>,
    /// The working directory: the package, so `--import tsx` resolves the
    /// package's own tsx.
    pub cwd: PathBuf,
}

impl ServerStartCommand {
    /// The command as the person would type it, for failure messages.
    pub fn display(&self) -> String {
        let mut text = self.program.display().to_string();
        for arg in &self.args {
            text.push(' ');
            text.push_str(&arg.display().to_string());
        }
        text
    }
}

pub fn server_start_command(package: &Path) -> ServerStartCommand {
    let built = package.join("dist").join("viewer").join("cli.js");
    if built.is_file() {
        return ServerStartCommand {
            program: PathBuf::from("node"),
            args: vec![built, PathBuf::from("server"), PathBuf::from("start")],
            cwd: package.to_path_buf(),
        };
    }
    let source = package.join("src").join("viewer").join("cli.ts");
    ServerStartCommand {
        program: PathBuf::from("node"),
        args: vec![
            PathBuf::from("--import"),
            PathBuf::from("tsx"),
            source,
            PathBuf::from("server"),
            PathBuf::from("start"),
        ],
        cwd: package.to_path_buf(),
    }
}

/// Run the package's own start command and wait for it. `Ok(())` when the
/// CLI exited successfully, which is its ready handshake: `server start`
/// answers only once the server replies to `server.status`. `Err` carries
/// the server's own error text (stderr, or the spawn failure) for the
/// transcript. One attempt; the retry loop inside the CLI is the package's
/// own bounded one, not a second one here.
pub async fn run_server_start(command: &ServerStartCommand) -> std::result::Result<(), String> {
    let mut process = match tokio::process::Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(process) => process,
        Err(err) => {
            return Err(format!("could not run `{}`: {err}", command.display()));
        }
    };
    let output = match tokio::time::timeout(SERVER_START_TIMEOUT, process.wait_with_output()).await
    {
        Ok(output) => output,
        Err(_) => {
            // `kill_on_drop` stops the child when the dropped wait future
            // releases it.
            return Err(format!(
                "`{}` did not finish within {} s",
                command.display(),
                SERVER_START_TIMEOUT.as_secs()
            ));
        }
    };
    let output = match output {
        Ok(output) => output,
        Err(err) => return Err(format!("waiting for `{}` failed: {err}", command.display())),
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    let detail = if detail.is_empty() {
        format!("exited with {}", output.status)
    } else {
        detail.chars().take(2_000).collect()
    };
    Err(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal pi-workflows package directory: `package.json` with the
    /// package name.
    fn make_package(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create package dir");
        std::fs::write(
            dir.join("package.json"),
            format!("{{\"name\":\"{PIWORKFLOWS_PACKAGE_NAME}\",\"version\":\"0.0.0\"}}"),
        )
        .expect("write package.json");
        dir.to_path_buf()
    }

    fn write_settings(agent_dir: &Path, packages_json: &str) {
        std::fs::create_dir_all(agent_dir).expect("create agent dir");
        std::fs::write(
            agent_dir.join("settings.json"),
            format!("{{\"packages\":{packages_json}}}"),
        )
        .expect("write settings.json");
    }

    #[test]
    fn resolves_a_path_entry_to_a_piworkflows_checkout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let checkout = make_package(&dir.path().join("checkouts").join("pi-workflows"));
        let agent_dir = dir.path().join("agent");
        write_settings(
            &agent_dir,
            &format!("[\"npm:pi-typesafe\",{}]", json_string(&checkout)),
        );
        assert_eq!(resolve_piworkflows_package(&agent_dir), Ok(checkout));
    }

    #[test]
    fn resolves_the_npm_entry_under_the_agent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_dir = dir.path().join("agent");
        let installed = make_package(
            &agent_dir
                .join("npm")
                .join("node_modules")
                .join("@osolmaz")
                .join("pi-workflows"),
        );
        write_settings(&agent_dir, "[\"npm:@osolmaz/pi-workflows\"]");
        assert_eq!(resolve_piworkflows_package(&agent_dir), Ok(installed));
    }

    #[test]
    fn an_object_entry_with_a_source_resolves_the_same_way() {
        let dir = tempfile::tempdir().expect("tempdir");
        let checkout = make_package(&dir.path().join("wf"));
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(&agent_dir).expect("create agent dir");
        std::fs::write(
            agent_dir.join("settings.json"),
            format!(
                "{{\"packages\":[{{\"source\":{},\"prompts\":[\"-starter/prompts/p.md\"]}}]}}",
                json_string(&checkout)
            ),
        )
        .expect("write settings.json");
        assert_eq!(resolve_piworkflows_package(&agent_dir), Ok(checkout));
    }

    #[test]
    fn neither_entry_fails_with_the_paths_tried() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_dir = dir.path().join("agent");
        // A path entry that is not a pi-workflows package, and an npm entry
        // that is not installed.
        let not_piw = dir.path().join("not-piw");
        std::fs::create_dir_all(&not_piw).expect("create dir");
        std::fs::write(not_piw.join("package.json"), "{\"name\":\"other\"}").expect("write");
        write_settings(
            &agent_dir,
            &format!("[{},\"npm:@osolmaz/pi-workflows\"]", json_string(&not_piw)),
        );
        let Err(resolution) = resolve_piworkflows_package(&agent_dir) else {
            panic!("resolution must fail");
        };
        let npm_tried = agent_dir
            .join("npm")
            .join("node_modules")
            .join("@osolmaz")
            .join("pi-workflows");
        assert_eq!(resolution.tried, vec![not_piw.display().to_string(), npm_tried.display().to_string()]);
        assert_eq!(resolution.settings_path, agent_dir.join("settings.json"));
    }

    #[test]
    fn no_settings_or_no_packages_list_fails_with_nothing_tried() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty = dir.path().join("agent");
        std::fs::create_dir_all(&empty).expect("create agent dir");
        let Err(resolution) = resolve_piworkflows_package(&empty) else {
            panic!("resolution must fail");
        };
        assert!(resolution.tried.is_empty());

        let no_list = dir.path().join("agent2");
        std::fs::create_dir_all(&no_list).expect("create agent dir");
        std::fs::write(no_list.join("settings.json"), "{\"theme\":\"dark\"}").expect("write");
        let Err(resolution) = resolve_piworkflows_package(&no_list) else {
            panic!("resolution must fail");
        };
        assert!(resolution.tried.is_empty());
    }

    #[test]
    fn a_live_socket_never_needs_a_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("server.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        // A listener that accepts: the probe of this socket must say live,
        // so no start is attempted.
        assert_eq!(socket_presence_at(&socket), SocketPresence::Live);
        assert!(!SocketPresence::Live.needs_start());
        drop(listener);

        // A socket file with nothing accepting: the probe must say refused
        // (or missing, once the kernel removed the file), and both need a
        // start.
        let presence = socket_presence_at(&socket);
        assert!(matches!(
            presence,
            SocketPresence::Refused | SocketPresence::Missing
        ));
        assert!(presence.needs_start());

        let missing = dir.path().join("missing.sock");
        assert_eq!(socket_presence_at(&missing), SocketPresence::Missing);
        assert!(SocketPresence::Missing.needs_start());
    }

    #[test]
    fn the_start_command_uses_the_built_cli_when_dist_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package = make_package(&dir.path().join("pi-workflows"));
        let built = package.join("dist").join("viewer").join("cli.js");
        std::fs::create_dir_all(built.parent().unwrap()).expect("create dist");
        std::fs::write(&built, "// built cli").expect("write cli.js");
        let command = server_start_command(&package);
        assert_eq!(command.program, PathBuf::from("node"));
        assert_eq!(
            command.args,
            vec![built, PathBuf::from("server"), PathBuf::from("start")]
        );
        assert_eq!(command.cwd, package);
        assert!(command.display().ends_with("cli.js server start"));
    }

    #[test]
    fn the_start_command_uses_the_source_cli_through_tsx_when_dist_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package = make_package(&dir.path().join("pi-workflows"));
        let source = package.join("src").join("viewer").join("cli.ts");
        std::fs::create_dir_all(source.parent().unwrap()).expect("create src");
        std::fs::write(&source, "// source cli").expect("write cli.ts");
        let command = server_start_command(&package);
        assert_eq!(command.program, PathBuf::from("node"));
        assert_eq!(
            command.args,
            vec![
                PathBuf::from("--import"),
                PathBuf::from("tsx"),
                source,
                PathBuf::from("server"),
                PathBuf::from("start")
            ]
        );
        assert_eq!(command.cwd, package);
        assert!(command.display().contains("--import tsx"));
        assert!(command.display().ends_with("cli.ts server start"));
    }

    #[test]
    fn a_failed_start_reports_the_server_error_text() {
        // A command whose program cannot exist: the spawn failure itself is
        // the error text, and it names the command.
        let dir = tempfile::tempdir().expect("tempdir");
        let command = ServerStartCommand {
            program: dir.path().join("no-such-node"),
            args: vec![PathBuf::from("server"), PathBuf::from("start")],
            cwd: dir.path().to_path_buf(),
        };
        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(run_server_start(&command));
        assert!(err.is_err());
        let err = err.unwrap_err();
        assert!(err.contains("no-such-node"), "error text was: {err}");
        assert!(err.contains("server start"), "error text was: {err}");
    }

    fn json_string(path: &Path) -> String {
        serde_json::to_string(&path.display().to_string()).expect("serialize path")
    }
}
