//! Spike S5: does macOS Local Network permission follow the receiver connection into
//! a Control API server that is nested in a signed app bundle?
//!
//! This program is a stand-in for the GUI. As the main executable of a test bundle it
//! does what the planned launcher does: it starts the nested `denon-avr-api-server`
//! with `--data-dir DIR --exit-with-parent`, a held standard-input pipe, and standard
//! error in a file. It then asks that server, over the Operator socket, to do the things
//! the permission governs: open a TCP connection to the receiver, and run an SSDP scan.
//! Everything it sees goes to a result file and to standard output.
//!
//! With `--drive` it starts nothing and only drives a server someone else started, for
//! the terminal cases. Standard library only; it sends plain HTTP/1.1 itself.
//!
//! See docs/research/local-network-permission-server-macos.md for how to run it.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const SERVER_NAME: &str = "denon-avr-api-server";
/// How long the state read keeps trying, so that the person has time to answer a
/// permission prompt while the run is going: 18 attempts, 5 seconds apart. The
/// number can be lowered with `S5_ATTEMPTS`, for testing the program itself.
const STATE_PAUSE: Duration = Duration::from_secs(5);

fn state_attempts() -> u32 {
    std::env::var("S5_ATTEMPTS")
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(18)
}

struct Args {
    label: String,
    host: String,
    data_dir: PathBuf,
    /// `None` means drive a server that is already running.
    server: Option<PathBuf>,
    root: PathBuf,
}

impl Args {
    /// A data directory named for one case under a report named for another is a typing
    /// mistake that would hide which case a result belongs to.
    fn mismatch(&self) -> Option<String> {
        let named = self
            .data_dir
            .file_name()?
            .to_str()?
            .strip_prefix("d-")?
            .to_owned();
        (named != self.label).then(|| {
            format!(
                "NOTE: the label is {:?} but the data directory is d-{named}; check which case this is",
                self.label
            )
        })
    }

    fn parse() -> Result<Self, String> {
        let root = PathBuf::from(std::env::var("S5_ROOT").unwrap_or_else(|_| "/tmp/s5".into()));
        let mut label = None;
        let mut host = None;
        let mut data_dir = None;
        let mut server = None;
        let mut drive = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
            match arg.as_str() {
                "--label" => label = Some(value("--label")?),
                "--host" => host = Some(value("--host")?),
                "--data-dir" => data_dir = Some(PathBuf::from(value("--data-dir")?)),
                "--server" => server = Some(PathBuf::from(value("--server")?)),
                "--drive" => drive = true,
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let label = label.unwrap_or_else(|| label_from_bundle(&exe));
        let host = match host {
            Some(host) => host,
            None => read_config_host(&root)?,
        };
        let data_dir = data_dir.unwrap_or_else(|| root.join(format!("d-{label}")));
        let server = if drive {
            None
        } else {
            Some(server.unwrap_or_else(|| exe.with_file_name(SERVER_NAME)))
        };
        Ok(Self {
            label,
            host,
            data_dir,
            server,
            root,
        })
    }
}

/// `…/S5-same.app/Contents/MacOS/s5-host` is the case `same`; anything else is
/// `terminal`. A bundle in a directory whose name ends in `-copy` (the install-flow test's
/// copy, see `--copy` in build-bundles.sh) gets `-copy` added, so that its report, data
/// directory and notes are not the original's, which has the same file name. The window
/// (`tools/s5/app/main.swift`) makes the same label.
fn label_from_bundle(exe: &Path) -> String {
    exe.ancestors()
        .find_map(|dir| {
            let label = dir
                .file_name()?
                .to_str()?
                .strip_suffix(".app")?
                .strip_prefix("S5-")?;
            let copied = dir.parent()?.file_name()?.to_str()?.ends_with("-copy");
            Some(if copied {
                format!("{label}-copy")
            } else {
                label.to_owned()
            })
        })
        .unwrap_or_else(|| "terminal".to_owned())
}

fn read_config_host(root: &Path) -> Result<String, String> {
    let path = root.join("config.txt");
    let text = fs::read_to_string(&path).map_err(|e| {
        format!(
            "no --host and no {}: {e}. Write a line `host=<receiver address>` there",
            path.display()
        )
    })?;
    text.lines()
        .find_map(|line| line.trim().strip_prefix("host="))
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
        .ok_or_else(|| format!("{} has no `host=` line", path.display()))
}

struct Report {
    file: File,
}

/// An earlier report of the same name is kept, under its own time, and not overwritten.
fn keep_earlier(path: &Path) {
    if !path.exists() {
        return;
    }
    let stamp = fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("result");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("txt");
    let _ = fs::rename(
        path,
        path.with_file_name(format!("{stem}.{stamp}.{extension}")),
    );
}

impl Report {
    fn new(path: &Path) -> Self {
        keep_earlier(path);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap_or_else(|e| {
                eprintln!("s5-host: cannot write {}: {e}", path.display());
                std::process::exit(2)
            });
        Self { file }
    }

    /// Written at once, so a run that hangs still leaves what it had found.
    fn line(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        println!("{text}");
        let _ = writeln!(self.file, "{text}");
        let _ = self.file.flush();
    }
}

fn capture(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(out) => format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => format!("({program}: {e})"),
    }
}

/// The wall-clock time, so that a line of the report can be matched to the screen recording and
/// to the notes the window's buttons write.
fn clock() -> String {
    capture("/bin/date", &["+%H:%M:%S"]).trim().to_owned()
}

/// The executables whose presence during a run matters: a bundle left running (and, with the
/// same bundle id, brought forward by `open` instead of started again), a server that still
/// holds the receiver's one control connection, or the installed app.
const WATCHED: [&str; 4] = [
    "s5-app",
    "s5-host",
    "denon-avr-api-server",
    "denon-avr-remote-gui",
];

/// The watched programs in `ps -axo pid=,ppid=,comm=` output, other than this program and its
/// parent. A program is matched on the base name of its executable, never on a whole command
/// line: the repository's own path contains `denon-avr`, and the `open -W` that waits for this
/// run in the terminal names the bundle.
fn other_processes(ps_output: &str, own: u32, parent: u32) -> Vec<String> {
    ps_output
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid: u32 = words.next()?.parse().ok()?;
            let ppid: u32 = words.next()?.parse().ok()?;
            let executable = words.collect::<Vec<_>>().join(" ");
            let base = executable.rsplit('/').next()?;
            (pid != own && pid != parent && WATCHED.contains(&base))
                .then(|| format!("    pid {pid} (parent {ppid}) {executable}"))
        })
        .collect()
}

const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_UUID: u32 = 0x1b;
/// The size of a 64-bit Mach-O header, where the load commands begin.
const MACHO_HEADER: usize = 32;

/// One little-endian 32-bit word of a Mach-O header, or an error if the file ends before it.
fn word(header: &[u8], at: usize) -> Result<u32, String> {
    header
        .get(at..at + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| "the file ends inside the Mach-O header".to_owned())
}

/// The header and the load commands of a thin, 64-bit, little-endian Mach-O file: the part
/// of the file in which the UUID lies. Anything else (a script, a fat binary) is an error.
fn macho_header(file: &File) -> Result<Vec<u8>, String> {
    let mut start = [0u8; MACHO_HEADER];
    file.read_exact_at(&mut start, 0)
        .map_err(|e| format!("cannot read a Mach-O header: {e}"))?;
    match word(&start, 0)? {
        MH_MAGIC_64 => {}
        // FAT_MAGIC and FAT_MAGIC_64, read as little-endian words.
        0xbeba_feca | 0xbfba_feca => {
            return Err("a fat (universal) binary; only thin arm64 programs are handled".into())
        }
        _ => return Err("not a 64-bit little-endian Mach-O file".into()),
    }
    let commands_size = word(&start, 20)? as usize;
    if commands_size > 1 << 20 {
        return Err("the load commands are larger than any real program's".into());
    }
    let mut header = vec![0u8; MACHO_HEADER + commands_size];
    file.read_exact_at(&mut header, 0)
        .map_err(|e| format!("cannot read the load commands: {e}"))?;
    Ok(header)
}

/// Where the 16 bytes of the UUID lie in `header` (the result of `macho_header`).
fn uuid_offset(header: &[u8]) -> Result<usize, String> {
    let mut at = MACHO_HEADER;
    for _ in 0..word(header, 16)? {
        let kind = word(header, at)?;
        let size = word(header, at + 4)? as usize;
        if size < 8 {
            return Err("a load command is shorter than its own header".into());
        }
        if kind == LC_UUID {
            if size < 24 || at + 24 > header.len() {
                return Err("the LC_UUID load command is cut short".into());
            }
            return Ok(at + 8);
        }
        at += size;
    }
    Err("no LC_UUID load command".into())
}

/// `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`, as `dwarfdump --uuid` prints it.
fn format_uuid(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The UUID of a Mach-O program: the identifier a link gives its output, and which a new
/// release of the program has again.
fn macho_uuid(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let header = macho_header(&file)?;
    let at = uuid_offset(&header)?;
    Ok(format_uuid(&header[at..at + 16]))
}

/// Gives a Mach-O program a new random UUID, in place, and returns the old and the new one.
/// The signature is invalid afterwards: the caller signs the program again. The point is that
/// two builds of one identity should differ as two releases do (a link gives every output
/// its own UUID), and that two bundles should not share a program's UUID.
fn retag_uuid(path: &Path) -> Result<(String, String), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let header = macho_header(&file)?;
    let at = uuid_offset(&header)?;
    let old = format_uuid(&header[at..at + 16]);
    let mut fresh = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut fresh))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    fresh[6] = (fresh[6] & 0x0f) | 0x40;
    fresh[8] = (fresh[8] & 0x3f) | 0x80;
    file.write_all_at(&fresh, at as u64)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((old, format_uuid(&fresh)))
}

/// One report line saying a program's UUID, or why it cannot be read.
fn uuid_line(name: &str, path: &Path) -> String {
    match macho_uuid(path) {
        Ok(uuid) => format!("    UUID {name}: {uuid}"),
        Err(e) => format!("    UUID {name}: unreadable ({e})"),
    }
}

/// The lines of `codesign -dvvv` that say whose identity a binary has.
fn signature_summary(path: &Path) -> String {
    let text = capture("/usr/bin/codesign", &["-dvvv", &path.to_string_lossy()]);
    let wanted = [
        "Executable=",
        "Identifier=",
        "Format=",
        "CodeDirectory",
        "Signature",
        "TeamIdentifier=",
        "CDHash=",
        "Authority=",
    ];
    text.lines()
        .filter(|line| wanted.iter().any(|w| line.starts_with(w)))
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The chain of parent processes, from this one up to launchd or a terminal. It shows how
/// the program was started, and so which application macOS holds responsible for it.
fn ancestry() -> String {
    let mut lines = Vec::new();
    let mut pid = std::process::id();
    for _ in 0..8 {
        let out = capture(
            "/bin/ps",
            &["-o", "pid=,ppid=,command=", "-p", &pid.to_string()],
        );
        let line = out.trim().to_owned();
        if line.is_empty() {
            break;
        }
        let parent: Option<u32> = line.split_whitespace().nth(1).and_then(|p| p.parse().ok());
        lines.push(format!("    {line}"));
        match parent {
            Some(parent) if parent > 1 && parent != pid => pid = parent,
            _ => break,
        }
    }
    lines.join("\n")
}

/// Everything but the unreserved characters of RFC 3986, as the contract does.
fn encode_segment(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn request(
    socket: &Path,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<(u16, String), String> {
    let mut stream =
        UnixStream::connect(socket).map_err(|e| format!("connect {}: {e}", socket.display()))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let body = body.unwrap_or("");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: s5\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if method != "GET" {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    head.push_str(body);
    stream
        .write_all(head.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read (after {} bytes): {e}", raw.len()))?;
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the response has no end of head")?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("the response has no status")?;
    let body = &raw[split + 4..];
    let bytes = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked(body)?
    } else {
        body.to_vec()
    };
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

fn decode_chunked(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("a chunk has no size line")?;
        let size_text =
            std::str::from_utf8(&body[..end]).map_err(|_| "a chunk size is not text")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "a chunk size is not hexadecimal")?;
        body = &body[end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err("a chunk is short".into());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// The text of a string field in compact JSON, without a JSON library.
fn json_string(body: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = body.find(&marker)? + marker.len();
    let rest = &body[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

fn snippet(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 300 {
        format!("{}…", one_line.chars().take(300).collect::<String>())
    } else {
        one_line
    }
}

/// Words that mean the operating system, not the receiver, refused the connection.
fn looks_blocked(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "no route to host",
        "unreachable",
        "ehostunreach",
        "operation not permitted",
    ]
    .iter()
    .any(|w| text.contains(w))
}

/// Only a directory made for this spike is ever cleared.
fn clear_scratch(root: &Path, dir: &Path) {
    if dir.starts_with(root) && dir != root && dir.components().count() >= 3 {
        let _ = fs::remove_dir_all(dir);
    }
}

/// `--retag-uuid FILE` and `--print-uuid FILE`: what `tools/s5/build-bundles.sh` uses on the
/// copies in a bundle. They start no server and write no report. `None` means neither was
/// asked for.
fn uuid_command() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    let command = args.next()?;
    if command != "--retag-uuid" && command != "--print-uuid" {
        return None;
    }
    let Some(path) = args.next() else {
        eprintln!("s5-host: {command} needs a file");
        return Some(2);
    };
    let path = Path::new(&path);
    let result = if command == "--retag-uuid" {
        retag_uuid(path).map(|(old, new)| format!("{old} -> {new}"))
    } else {
        macho_uuid(path)
    };
    Some(match result {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            eprintln!("s5-host: {command} {}: {e}", path.display());
            2
        }
    })
}

fn main() {
    if let Some(status) = uuid_command() {
        std::process::exit(status);
    }
    let args = match Args::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("s5-host: {e}");
            std::process::exit(2);
        }
    };
    let _ = fs::create_dir_all(&args.root);
    let mut report = Report::new(&args.root.join(format!("result-{}.txt", args.label)));
    let exe = std::env::current_exe().unwrap_or_default();

    report.line(format!("S5 run: {}", args.label));
    report.line("Leave this running until the SUMMARY line appears (about 90 s if it is blocked).");
    report.line("If a permission dialog appears, write down its exact name and answer it.");
    if let Some(note) = args.mismatch() {
        report.line(note);
    }
    report.line(format!(
        "macOS {} on {}",
        capture("/usr/bin/sw_vers", &["-productVersion"]).trim(),
        capture("/usr/bin/uname", &["-m"]).trim()
    ));
    report.line(format!("receiver address: {}", args.host));
    report.line(format!("data directory: {}", args.data_dir.display()));
    report.line(format!(
        "host pid {}, {}",
        std::process::id(),
        exe.display()
    ));
    report.line(signature_summary(&exe));
    // The programs' UUIDs: bundles built from the same programs share them unless retagged.
    report.line(uuid_line("s5-host", &exe));
    let app = exe.with_file_name("s5-app");
    if app.exists() {
        report.line(uuid_line("s5-app", &app));
    }
    report.line("started by (pid, parent pid, command), this program first:");
    report.line(ancestry());
    report.line(format!(
        "started at {} {}",
        capture("/bin/date", &["+%Y-%m-%d"]).trim(),
        clock()
    ));

    // Spawn mode: what the launcher will do (architecture, "The algorithm", step 3).
    let mut child = None;
    let mut held_pipe = None;
    let log_path = args.root.join(format!("server-{}.log", args.label));
    if let Some(server) = &args.server {
        report.line(format!("server: {}", server.display()));
        report.line(signature_summary(server));
        report.line(uuid_line("server", server));
        // Nothing else of ours should be running yet; in drive mode a server is expected.
        // -ww: never cut a long path short, or a leftover server would lose its base name.
        let listing = capture("/bin/ps", &["-ww", "-axo", "pid=,ppid=,comm="]);
        let others = other_processes(
            &listing,
            std::process::id(),
            std::os::unix::process::parent_id(),
        );
        let listed = listing.lines().any(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|word| word.parse::<u32>().is_ok())
        });
        if !listed {
            report.line(format!(
                "WARN cannot tell whether other S5 or Denon programs are running: ps said {}",
                snippet(&listing)
            ));
        } else if others.is_empty() {
            report.line("PASS clear: no other S5 or Denon program is running");
        } else {
            report.line(
                "WARN other S5 or Denon programs are running; one may hold the receiver, or may be what `open` brought forward instead of starting this bundle:",
            );
            for line in others {
                report.line(line);
            }
        }
        clear_scratch(&args.root, &args.data_dir);
        keep_earlier(&log_path);
        let log = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .expect("the server log");
        let mut spawned = match Command::new(server)
            .arg("--data-dir")
            .arg(&args.data_dir)
            .arg("--exit-with-parent")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
        {
            Ok(spawned) => spawned,
            Err(e) => {
                report.line(format!("FAIL start: cannot run the server: {e}"));
                std::process::exit(1);
            }
        };
        held_pipe = spawned.stdin.take();
        report.line(format!("server pid {}", spawned.id()));
        child = Some(spawned);
    }

    let socket = args.data_dir.join("run/operator.sock");
    let token_file = args.data_dir.join("credentials/operator.token");

    // Ready: the socket accepts (the launcher's probe), within 20 seconds.
    let started = Instant::now();
    let mut ready = false;
    while started.elapsed() < Duration::from_secs(20) {
        if let Some(spawned) = child.as_mut() {
            if let Ok(Some(status)) = spawned.try_wait() {
                report.line(format!("FAIL start: the server exited early: {status}"));
                tail_log(&mut report, &log_path);
                std::process::exit(1);
            }
        }
        if UnixStream::connect(&socket).is_ok() {
            ready = true;
            break;
        }
        sleep(Duration::from_millis(100));
    }
    if !ready {
        report.line(format!(
            "FAIL start: nothing accepted on {}",
            socket.display()
        ));
        tail_log(&mut report, &log_path);
        std::process::exit(1);
    }
    report.line(format!(
        "PASS start: serving after {:.1} s",
        started.elapsed().as_secs_f32()
    ));

    let token = match fs::read_to_string(&token_file) {
        Ok(text) => text.trim().to_owned(),
        Err(e) => {
            report.line(format!("FAIL token: {}: {e}", token_file.display()));
            std::process::exit(1);
        }
    };

    let short = Duration::from_secs(15);
    let mut state_ok = false;
    match request(&socket, &token, "GET", "/v1/health", None, short) {
        Ok((200, body)) => report.line(format!("PASS health: {}", snippet(&body))),
        Ok((status, body)) => report.line(format!("FAIL health: {status} {}", snippet(&body))),
        Err(e) => report.line(format!("FAIL health: {e}")),
    }

    let adhoc_body = format!("{{\"host\":\"{}\"}}", args.host);
    let id = match request(
        &socket,
        &token,
        "POST",
        "/v1/receivers/ad-hoc",
        Some(&adhoc_body),
        short,
    ) {
        Ok((200, body)) => json_string(&body, "id"),
        Ok((status, body)) => {
            report.line(format!("FAIL ad-hoc: {status} {}", snippet(&body)));
            None
        }
        Err(e) => {
            report.line(format!("FAIL ad-hoc: {e}"));
            None
        }
    };
    if let Some(id) = &id {
        report.line(format!("PASS ad-hoc: receiver id {id}"));
        // The receiver connection. Retried so that a permission prompt can be
        // answered while the run is going.
        let path = format!("/v1/receivers/{}/state", encode_segment(id));
        let mut first_failure = None;
        let mut passed_at = None;
        let attempts = state_attempts();
        for attempt in 1..=attempts {
            let when = started.elapsed().as_secs();
            match request(&socket, &token, "GET", &path, None, Duration::from_secs(40)) {
                Ok((200, _)) => {
                    report.line(format!(
                        "PASS state: the server reached the receiver (attempt {attempt}, {when} s in, at {})",
                        clock()
                    ));
                    state_ok = true;
                    passed_at = Some(attempt);
                    break;
                }
                Ok((status, body)) => {
                    let note = if looks_blocked(&body) {
                        "  <- looks like the OS refused it"
                    } else {
                        ""
                    };
                    report.line(format!(
                        "  attempt {attempt} ({when} s, {}): {status} {}{note}",
                        clock(),
                        snippet(&body)
                    ));
                    first_failure.get_or_insert(format!("{status} {}", snippet(&body)));
                }
                Err(e) => {
                    report.line(format!("  attempt {attempt} ({when} s, {}): {e}", clock()));
                    first_failure.get_or_insert(e);
                }
            }
            if attempt < attempts {
                if attempt == 1 {
                    report.line(format!(
                        "  (if a permission dialog is showing, answer it now; the run keeps trying for about {} s)",
                        u64::from(attempts) * STATE_PAUSE.as_secs()
                    ));
                }
                sleep(STATE_PAUSE);
            }
        }
        if let Some(attempt) = passed_at.filter(|attempt| *attempt > 1) {
            report.line(format!(
                "NOTE: state was refused {} times and then passed at attempt {attempt}: either a permission dialog was answered during the run, or the system checked a changed program silently (run 6's rebuilt bundle did that, with no dialog); match the time with the window's notes and the screenshots",
                attempt - 1
            ));
        }
        if !state_ok {
            report.line(format!(
                "FAIL state: no success in {attempts} attempts; first failure: {}",
                first_failure.unwrap_or_default()
            ));
        }
    }

    // The SSDP scan: UDP multicast, which the same permission governs.
    match request(
        &socket,
        &token,
        "POST",
        "/v1/receivers/discover",
        Some("{\"timeout_ms\":5000}"),
        Duration::from_secs(30),
    ) {
        Ok((200, body)) => {
            let found = body.contains(&format!("\"host\":\"{}\"", args.host));
            let any = body.contains("\"host\"");
            if found {
                report.line("PASS discovery: the scan found the receiver");
            } else if any {
                report.line(format!(
                    "WARN discovery: found other devices but not {}: {}",
                    args.host,
                    snippet(&body)
                ));
            } else {
                report.line(format!("WARN discovery: found nothing: {}", snippet(&body)));
            }
        }
        Ok((status, body)) => report.line(format!("FAIL discovery: {status} {}", snippet(&body))),
        Err(e) => report.line(format!("FAIL discovery: {e}")),
    }

    // Stop what was started: the pipe is the whole instruction.
    if let Some(mut spawned) = child {
        drop(held_pipe);
        let waited = Instant::now();
        let status = loop {
            match spawned.try_wait() {
                Ok(Some(status)) => break format!("{status}"),
                Ok(None) if waited.elapsed() < Duration::from_secs(15) => {
                    sleep(Duration::from_millis(100))
                }
                Ok(None) => break "still running after 15 s".to_owned(),
                Err(e) => break format!("{e}"),
            }
        };
        report.line(format!("server stopped after the pipe closed: {status}"));
        tail_log(&mut report, &log_path);
    }

    report.line(format!(
        "SUMMARY {}: state={}",
        args.label,
        if state_ok { "PASS" } else { "FAIL" }
    ));
    report.line(format!(
        "(this report is {}/result-{}.txt)",
        args.root.display(),
        args.label
    ));
    std::process::exit(if state_ok { 0 } else { 1 });
}

fn tail_log(report: &mut Report, path: &Path) {
    if let Ok(text) = fs::read_to_string(path) {
        let lines: Vec<&str> = text.lines().collect();
        report.line(format!(
            "server log (last {} of {} lines):",
            lines.len().min(25),
            lines.len()
        ));
        for line in lines.iter().skip(lines.len().saturating_sub(25)) {
            report.line(format!("    {line}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_stays_in_one_path_segment() {
        assert_eq!(encode_segment("adhoc:host.example"), "adhoc%3Ahost.example");
        assert_eq!(encode_segment("a/b c?d%e"), "a%2Fb%20c%3Fd%25e");
    }

    #[test]
    fn a_plain_response_is_split_into_status_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"id\":\"x:1\"}";
        let (status, body) = parse_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(json_string(&body, "id").as_deref(), Some("x:1"));
    }

    #[test]
    fn a_chunked_response_is_joined() {
        let raw = b"HTTP/1.1 503 Service Unavailable\r\ntransfer-encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let (status, body) = parse_response(raw).unwrap();
        assert_eq!((status, body.as_str()), (503, "hello world"));
    }

    #[test]
    fn a_truncated_response_is_an_error() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\ncontent-le").is_err());
        assert!(decode_chunked(b"zz\r\nhi").is_err());
    }

    #[test]
    fn the_blocked_words_are_found_whatever_the_case() {
        assert!(looks_blocked("connect: No route to host (os error 65)"));
        assert!(!looks_blocked("connection refused"));
    }

    #[test]
    fn the_label_comes_from_the_bundle_name() {
        let exe = Path::new("/x/target/s5/S5-same.app/Contents/MacOS/s5-host");
        assert_eq!(label_from_bundle(exe), "same");
        assert_eq!(
            label_from_bundle(Path::new("/usr/local/bin/s5-host")),
            "terminal"
        );
        // The install-flow test's copy has the original's file name in another directory.
        let copy = Path::new("/x/target/s5-copy/S5-own-180617.app/Contents/MacOS/s5-host");
        assert_eq!(label_from_bundle(copy), "own-180617-copy");
        let original = Path::new("/x/target/s5/S5-own-180617.app/Contents/MacOS/s5-host");
        assert_eq!(label_from_bundle(original), "own-180617");
        // Another app on the way up is not an S5 bundle.
        let other = Path::new("/Applications/Denon AVR Remote.app/Contents/MacOS/s5-host");
        assert_eq!(label_from_bundle(other), "terminal");
    }

    #[test]
    fn an_earlier_report_is_kept_under_its_own_time() {
        let dir = std::env::temp_dir().join("s5-keep-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let report = dir.join("result-x.txt");
        fs::write(&report, "first").unwrap();
        keep_earlier(&report);
        assert!(!report.exists());
        let kept: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(fs::read_to_string(kept[0].path()).unwrap(), "first");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_earlier_server_log_keeps_its_extension() {
        let dir = std::env::temp_dir().join("s5-keep-log-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("server-x.log");
        fs::write(&log, "first").unwrap();
        keep_earlier(&log);
        assert!(!log.exists());
        let kept: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        let name = kept[0].file_name().into_string().unwrap();
        assert!(
            name.starts_with("server-x.") && name.ends_with(".log"),
            "{name}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_programs_are_found_by_executable_name_only() {
        let repo = "/Volumes/Work_Volume/denon-avr-remote-multi-platform";
        let ps = format!(
            "    1     0 /sbin/launchd\n\
             \x20 410   409 -zsh\n\
             \x20 500   410 /usr/bin/open\n\
             \x20 600     1 {repo}/target/s5/S5-own-1.app/Contents/MacOS/s5-app\n\
             \x20 601   600 {repo}/target/s5/S5-own-1.app/Contents/MacOS/s5-host\n\
             \x20 700     1 /Applications/Denon AVR Remote.app/Contents/MacOS/denon-avr-remote-gui\n\
             \x20 800     1 {repo}/target/s5/S5-same-0.app/Contents/MacOS/denon-avr-api-server\n\
             \x20 900   410 /usr/bin/grep\n"
        );
        // This program is pid 601 and its parent is 600: neither is "other", and neither are
        // the shell, the `open` that waits for the run, or anything that merely has the
        // repository in its path.
        let others = other_processes(&ps, 601, 600);
        assert_eq!(others.len(), 2, "{others:?}");
        assert!(others[0].contains("pid 700") && others[0].contains("Denon AVR Remote.app"));
        assert!(others[1].contains("pid 800") && others[1].contains("denon-avr-api-server"));
        // With nothing else running there is nothing to report, and junk lines are skipped.
        assert!(other_processes("not a line\n 1 0 /sbin/launchd\n", 5, 4).is_empty());
        // A bundle left over from an earlier run is reported when it is not this one.
        assert_eq!(other_processes(&ps, 5, 4).len(), 4);
    }

    /// A thin arm64 Mach-O file with a segment command (so that the UUID is not first), an
    /// `LC_UUID` command if `uuid` is given, and some bytes of "code".
    fn fake_macho(uuid: Option<[u8; 16]>) -> Vec<u8> {
        let mut commands = Vec::new();
        commands.extend_from_slice(&0x19u32.to_le_bytes());
        commands.extend_from_slice(&72u32.to_le_bytes());
        commands.extend_from_slice(&[0xAA; 64]);
        let mut count = 1u32;
        if let Some(uuid) = uuid {
            commands.extend_from_slice(&LC_UUID.to_le_bytes());
            commands.extend_from_slice(&24u32.to_le_bytes());
            commands.extend_from_slice(&uuid);
            count += 1;
        }
        let mut file = Vec::new();
        file.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        file.extend_from_slice(&0x0100_000cu32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&2u32.to_le_bytes());
        file.extend_from_slice(&count.to_le_bytes());
        file.extend_from_slice(&(commands.len() as u32).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&commands);
        file.extend_from_slice(&[0x5C; 200]);
        file
    }

    fn scratch_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("s5-uuid-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("program");
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_uuid_is_read_and_written_as_dwarfdump_does() {
        let mut uuid = [0u8; 16];
        for (i, byte) in uuid.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let path = scratch_file("read", &fake_macho(Some(uuid)));
        assert_eq!(
            macho_uuid(&path).unwrap(),
            "00010203-0405-0607-0809-0A0B0C0D0E0F"
        );
        assert!(uuid_line("server", &path).ends_with("00010203-0405-0607-0809-0A0B0C0D0E0F"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn retagging_changes_the_sixteen_uuid_bytes_and_nothing_else() {
        let before = fake_macho(Some([7u8; 16]));
        let path = scratch_file("retag", &before);
        let (old, new) = retag_uuid(&path).unwrap();
        assert_eq!(old, "07070707-0707-0707-0707-070707070707");
        assert_ne!(old, new);
        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        // 32 header bytes, the 72-byte segment command, then the 8-byte LC_UUID header.
        let at = 32 + 72 + 8;
        for (i, (a, b)) in before.iter().zip(&after).enumerate() {
            if !(at..at + 16).contains(&i) {
                assert_eq!(a, b, "byte {i} changed");
            }
        }
        assert_eq!(macho_uuid(&path).unwrap(), new);
        // A random version-4 UUID, as a link would not make but as any reader accepts.
        assert_eq!(after[at + 6] & 0xf0, 0x40);
        assert_eq!(after[at + 8] & 0xc0, 0x80);
        // Two retags of one program do not give one UUID.
        let (_, again) = retag_uuid(&path).unwrap();
        assert_ne!(again, new);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn anything_but_a_thin_macho_with_a_uuid_is_refused_and_left_alone() {
        let mut fat = vec![0xCA, 0xFE, 0xBA, 0xBE];
        fat.resize(64, 0);
        let mut truncated = fake_macho(Some([1u8; 16]));
        truncated.truncate(60);
        let cases: [(&str, Vec<u8>, &str); 5] = [
            ("empty", Vec::new(), "cannot read a Mach-O header"),
            (
                "script",
                b"#!/bin/sh\necho this is a script, longer than a header\n".to_vec(),
                "not a 64-bit little-endian Mach-O",
            ),
            ("fat", fat, "fat (universal)"),
            ("no-uuid", fake_macho(None), "no LC_UUID"),
            ("truncated", truncated, "cannot read the load commands"),
        ];
        for (name, bytes, expected) in cases {
            let path = scratch_file(name, &bytes);
            let error = retag_uuid(&path).unwrap_err();
            assert!(error.contains(expected), "{name}: {error}");
            assert!(macho_uuid(&path).is_err(), "{name}");
            assert_eq!(fs::read(&path).unwrap(), bytes, "{name} was changed");
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
        assert!(retag_uuid(Path::new("/nonexistent/s5/program")).is_err());
        assert!(uuid_line("x", Path::new("/nonexistent/s5/program")).contains("unreadable"));
    }

    #[test]
    fn a_label_that_disagrees_with_the_data_directory_is_noted() {
        let args = |label: &str, dir: &str| Args {
            label: label.into(),
            host: "h".into(),
            data_dir: PathBuf::from(dir),
            server: None,
            root: PathBuf::from("/tmp/s5"),
        };
        assert!(args("t1", "/tmp/s5/d-t2").mismatch().is_some());
        assert!(args("t2", "/tmp/s5/d-t2").mismatch().is_none());
        assert!(args("x", "/somewhere/else").mismatch().is_none());
    }

    #[test]
    fn only_a_scratch_directory_under_the_root_is_cleared() {
        let root = std::env::temp_dir().join("s5-clear-test");
        let inside = root.join("d-x");
        fs::create_dir_all(&inside).unwrap();
        clear_scratch(&root, Path::new("/"));
        clear_scratch(&root, &root);
        assert!(root.exists());
        clear_scratch(&root, &inside);
        assert!(!inside.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
