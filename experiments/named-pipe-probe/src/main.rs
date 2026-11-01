//! Decision-gate probe for ADR 1 (named pipes, pid-file locks); not part of qex.
//! Usage: `named-pipe-probe [--out FILE]`; the other subcommands are its child roles.

use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, ptr, thread};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::*;

const WINDOWS_SYS: &str = "0.61.2";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("squatter" | "server" | "client" | "holder" | "reader" | "deleter") => agent(),
        Some("pair-server") => pair(&args[1..], true),
        Some("pair-client") => pair(&args[1..], false),
        _ => run_all(&args),
    }
}

// ---------------------------------------------------------------- output

static OUT: Mutex<Option<fs::File>> = Mutex::new(None);

fn out(line: &str) {
    let line = mask_sids(line);
    println!("{line}");
    if let Some(f) = OUT.lock().unwrap().as_mut() {
        f.write_all(format!("{line}\n").as_bytes()).unwrap();
    }
}

/// `S-1-5-21-a-b-c-RID` becomes `S-1-5-21-X-X-X-RID`: the machine part stays out of the logs.
fn mask_sids(s: &str) -> String {
    let (mut o, mut rest) = (String::new(), s);
    while let Some(i) = rest.find("S-1-5-21-") {
        o.push_str(&rest[..i]);
        let tail = &rest[i + 9..];
        let end = tail
            .find(|c: char| !(c.is_ascii_digit() || c == '-'))
            .unwrap_or(tail.len());
        let parts: Vec<&str> = tail[..end].split('-').collect();
        o.push_str("S-1-5-21-X-X-X");
        if parts.len() > 3 {
            o.push('-');
            o.push_str(&parts[3..].join("-"));
        }
        rest = &tail[end..];
    }
    o.push_str(rest);
    o
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn set_out(path: Option<String>) {
    if let Some(p) = path {
        *OUT.lock().unwrap() = Some(fs::File::create(p).expect("create --out file"));
    }
}

#[derive(Clone, Copy)]
enum Status {
    Pass,
    Fail,
    Recorded,
    NotEvaluated,
}

fn verdict(ok: bool) -> Status {
    if ok { Status::Pass } else { Status::Fail }
}

/// One line per check; the status is always the last token, for the acceptance greps.
fn check(id: &str, name: &str, call: &str, got: &str, expected: &str, status: Status) {
    let status = match status {
        Status::Pass => "PASS",
        Status::Fail => "FAIL",
        Status::Recorded => "RECORDED",
        Status::NotEvaluated => "NOT EVALUATED",
    };
    out(&format!(
        "[{id}] {name} | call={call} | got={got} | expected={expected} | {status}"
    ));
}

/// `ok` or `err:<code>` from an answer of `exec`.
fn st(r: &str) -> String {
    let mut w = r.split_whitespace();
    match w.next() {
        Some("ok") => "ok".into(),
        Some("err") => format!("err:{}", w.next().unwrap_or("?")),
        _ => format!("bad:{r}"),
    }
}

fn kv<'a>(r: &'a str, key: &str) -> &'a str {
    r.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .unwrap_or("")
}

fn ms(r: &str) -> u64 {
    kv(r, "ms").parse().unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------- header

fn run_all(args: &[String]) {
    set_out(arg(args, "--out"));
    header();
    p1();
    p2();
    p4();
}

fn header() {
    out("named-pipe-probe: decision-gate experiment for ADR 1; not part of qex");
    out(&format!("date_utc: {}", date_utc()));
    let crate_dir = exe_dir().join("..").join("..");
    let git = |a: &[&str]| run(Command::new("git").current_dir(&crate_dir).args(a));
    out(&format!("commit: {}", git(&["rev-parse", "HEAD"])));
    out(&format!(
        "src/main.rs git blob: {}  working copy: {}",
        git(&["rev-parse", "HEAD:experiments/named-pipe-probe/src/main.rs"]),
        git(&["hash-object", "src/main.rs"])
    ));
    let status = Command::new("git")
        .current_dir(&crate_dir)
        .args(["status", "--porcelain", "--", "."])
        .output();
    out(&format!(
        "probe source tree clean vs commit: {}",
        match status {
            Ok(o) if o.status.success() && o.stdout.is_empty() => "yes",
            Ok(o) if o.status.success() => "no",
            _ => "unknown",
        }
    ));
    out(&run(Command::new("cmd").args(["/c", "ver"])));
    out(&run(Command::new("rustc").arg("--version")));
    out(&format!("windows-sys: {WINDOWS_SYS}"));
    let me = own_token();
    out(&format!(
        "TokenElevation: {}  integrity level: {}",
        elevated(me) as u8,
        integrity(me)
    ));
    out(&format!("pipe names: {}<check>", pipe_name("")));
}

/// First non-empty stdout line, or `unknown`; stderr is never printed (it can hold paths).
fn run(c: &mut Command) -> String {
    c.output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(String::from)
        })
        .unwrap_or_else(|| "unknown".into())
}

fn date_utc() -> String {
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (days, sec) = (s.div_euclid(86400), s.rem_euclid(86400));
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sec / 3600,
        sec % 3600 / 60,
        sec % 60
    )
}

fn exe_dir() -> PathBuf {
    env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

/// Unique per run (pid and start time), so a crashed earlier run cannot collide.
fn pipe_name(tag: &str) -> String {
    static TICK: OnceLock<u128> = OnceLock::new();
    let tick = TICK.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    });
    format!(r"\\.\pipe\lrc-probe-{}-{tick}-{tag}", std::process::id())
}

// ---------------------------------------------------------------- Win32 helpers

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn last_err() -> u32 {
    unsafe { GetLastError() }
}

/// The one create call: every check passes this parameter set and differs only in `first`.
fn create_pipe(name: &str, max: u32, first: bool, sd: *mut c_void) -> Result<HANDLE, u32> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let flag = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    let h = unsafe {
        CreateNamedPipeW(
            wide(name).as_ptr(),
            PIPE_ACCESS_DUPLEX | flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            max,
            4096,
            4096,
            0,
            if sd.is_null() { ptr::null() } else { &sa },
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(last_err())
    } else {
        Ok(h)
    }
}

fn open_pipe(name: &str) -> Result<HANDLE, u32> {
    let h = unsafe {
        CreateFileW(
            wide(name).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(last_err())
    } else {
        Ok(h)
    }
}

fn own_token() -> HANDLE {
    let mut t = ptr::null_mut();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) };
    t
}

/// GetTokenInformation into a u64 buffer, so the structures in it are aligned.
fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<u64>, u32> {
    let mut len = 0u32;
    unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(last_err());
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let ok = unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 { Err(last_err()) } else { Ok(buf) }
}

fn sid_string(sid: PSID) -> String {
    let mut w = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut w) } == 0 {
        return format!("err:{}", last_err());
    }
    let mut n = 0;
    while unsafe { *w.add(n) } != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(w, n) });
    unsafe { LocalFree(w.cast()) };
    s
}

fn elevated(token: HANDLE) -> bool {
    token_info(token, TokenElevation)
        .map(|b| unsafe { (*b.as_ptr().cast::<TOKEN_ELEVATION>()).TokenIsElevated } != 0)
        .unwrap_or(false)
}

fn integrity(token: HANDLE) -> String {
    match token_info(token, TokenIntegrityLevel) {
        Ok(b) => sid_string(unsafe { (*b.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid }),
        Err(c) => format!("err:{c}"),
    }
}

// ---------------------------------------------------------------- the child roles

/// A child role: reads one command per line on stdin, answers one line on stdout.
fn agent() {
    let mut hs = Vec::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        println!("{}", exec(&mut hs, &line));
        std::io::stdout().flush().unwrap();
    }
}

fn hd(hs: &[usize], s: &str) -> HANDLE {
    hs[s.parse::<usize>().unwrap()] as HANDLE
}

fn keep(hs: &mut Vec<usize>, r: Result<HANDLE, u32>) -> String {
    match r {
        Ok(h) => {
            hs.push(h as usize);
            format!("ok h={}", hs.len() - 1)
        }
        Err(c) => format!("err {c}"),
    }
}

/// One Win32 step. Answers `ok [key=value ...]` or `err <code> [key=value ...]`.
fn exec(hs: &mut Vec<usize>, line: &str) -> String {
    let a: Vec<&str> = line.split_whitespace().collect();
    let done = |ok: bool| {
        if ok {
            "ok".to_string()
        } else {
            format!("err {}", last_err())
        }
    };
    unsafe {
        match a[0] {
            "pid" => format!("ok pid={}", std::process::id()),
            "create" => keep(
                hs,
                create_pipe(a[1], a[2].parse().unwrap(), a[3] == "1", ptr::null_mut()),
            ),
            "open" => keep(hs, open_pipe(a[1])),
            "connect" => done(ConnectNamedPipe(hd(hs, a[1]), ptr::null_mut()) != 0),
            "disconnect" => done(DisconnectNamedPipe(hd(hs, a[1])) != 0),
            "close" => done(CloseHandle(hd(hs, a[1])) != 0),
            "write" => {
                let (b, mut n) = (a[2].as_bytes(), 0);
                done(
                    WriteFile(
                        hd(hs, a[1]),
                        b.as_ptr(),
                        b.len() as u32,
                        &mut n,
                        ptr::null_mut(),
                    ) != 0,
                )
            }
            "read" => {
                let (mut buf, mut n) = ([0u8; 64], 0u32);
                if ReadFile(hd(hs, a[1]), buf.as_mut_ptr(), 64, &mut n, ptr::null_mut()) != 0 {
                    format!("ok data={}", String::from_utf8_lossy(&buf[..n as usize]))
                } else {
                    format!("err {}", last_err())
                }
            }
            "wait" => {
                let t = Instant::now();
                let ok = WaitNamedPipeW(wide(a[1]).as_ptr(), a[2].parse().unwrap()) != 0;
                let code = last_err();
                let ms = t.elapsed().as_millis();
                if ok {
                    format!("ok ms={ms}")
                } else {
                    format!("err {code} ms={ms}")
                }
            }
            "whoami" => format!("ok pid={} {}", std::process::id(), token_sids(own_token())),
            "serverid" => server_identity(hd(hs, a[1])),
            "owner" => pipe_owner(hd(hs, a[1])),
            "clientpid" => {
                let mut pid = 0;
                if GetNamedPipeClientProcessId(hd(hs, a[1]), &mut pid) != 0 {
                    format!("ok pid={pid}")
                } else {
                    format!("err {}", last_err())
                }
            }
            "createsd" => {
                let mut sd = ptr::null_mut();
                let sddl = wide(a[4]);
                let made = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sd,
                    ptr::null_mut(),
                );
                if made == 0 {
                    return format!("err {} step=ConvertStringSecurityDescriptor", last_err());
                }
                keep(
                    hs,
                    create_pipe(a[1], a[2].parse().unwrap(), a[3] == "1", sd),
                )
            }
            _ => "err unknown-command".into(),
        }
    }
}

fn token_sids(t: HANDLE) -> String {
    let user = token_info(t, TokenUser)
        .map(|b| sid_string(unsafe { (*b.as_ptr().cast::<TOKEN_USER>()).User.Sid }))
        .unwrap_or_else(|c| format!("err:{c}"));
    let owner = token_info(t, TokenOwner)
        .map(|b| sid_string(unsafe { (*b.as_ptr().cast::<TOKEN_OWNER>()).Owner }))
        .unwrap_or_else(|c| format!("err:{c}"));
    format!(
        "token_user={user} token_owner={owner} elevated={}",
        elevated(t) as u8
    )
}

/// Each step's raw code is kept, as the elevated pairings (P2e, P2f) record where the chain stops.
fn server_identity(h: HANDLE) -> String {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(h, &mut pid) } == 0 {
        return format!("err {} step=GetNamedPipeServerProcessId", last_err());
    }
    let p = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if p.is_null() {
        return format!("err {} pid={pid} step=OpenProcess", last_err());
    }
    let mut t = ptr::null_mut();
    let ok = unsafe { OpenProcessToken(p, TOKEN_QUERY, &mut t) };
    let code = last_err();
    unsafe { CloseHandle(p) };
    if ok == 0 {
        return format!("err {code} pid={pid} step=OpenProcessToken");
    }
    let s = format!("ok pid={pid} {}", token_sids(t));
    unsafe { CloseHandle(t) };
    s
}

/// GetSecurityInfo(OWNER) through a pipe handle; SE_FILE_OBJECT first, then SE_KERNEL_OBJECT.
fn pipe_owner(h: HANDLE) -> String {
    let mut tried = Vec::new();
    for (ty, name) in [
        (SE_FILE_OBJECT, "SE_FILE_OBJECT"),
        (SE_KERNEL_OBJECT, "SE_KERNEL_OBJECT"),
    ] {
        let (mut owner, mut sd) = (ptr::null_mut(), ptr::null_mut());
        let r = unsafe {
            GetSecurityInfo(
                h,
                ty,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut sd,
            )
        };
        if r == 0 {
            let s = sid_string(owner);
            unsafe { LocalFree(sd) };
            return format!("ok owner={s} type={name} {}", tried.join(" "));
        }
        tried.push(format!("{name}=err:{r}"));
    }
    format!("err tried {}", tried.join(" "))
}

// ---------------------------------------------------------------- spawn-role

/// A child process in one of the roles, driven one step at a time.
struct Agent {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<String>,
}

impl Agent {
    fn spawn(role: &str) -> Agent {
        let mut child = Command::new(env::current_exe().unwrap())
            .arg(role)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn a child role");
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Agent {
            child,
            input,
            lines,
        }
    }

    /// A step with no answer in 10 s comes back as `err timeout`, so a hung call cannot stall the run.
    fn cmd(&mut self, line: &str) -> String {
        if writeln!(self.input, "{line}")
            .and_then(|_| self.input.flush())
            .is_err()
        {
            return "err dead".into();
        }
        self.lines
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| "err timeout".into())
    }

    /// TerminateProcess, then wait until the process and its handles are gone.
    fn kill(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The probe process itself, taking the same steps as a child.
#[derive(Default)]
struct Local(Vec<usize>);

impl Local {
    fn cmd(&mut self, line: &str) -> String {
        exec(&mut self.0, line)
    }

    /// ConnectNamedPipe on another thread, as a listening server blocks in it.
    fn listen(&self, h: &str) -> thread::JoinHandle<String> {
        let h = hd(&self.0, h) as usize;
        thread::spawn(move || unsafe {
            if ConnectNamedPipe(h as HANDLE, ptr::null_mut()) != 0 {
                "ok".into()
            } else {
                format!("err {}", GetLastError())
            }
        })
    }
}

// ---------------------------------------------------------------- P1: who owns the name

fn p1() {
    let mut me = Local::default();
    let name = pipe_name("p1");
    let mut squatter = Agent::spawn("squatter");
    let sq = squatter.cmd(&format!("create {name} 255 0"));
    let flagged = me.cmd(&format!("create {name} 255 1"));
    check(
        "P1a",
        "FIRST_PIPE_INSTANCE vs a name another same-user process created first",
        "CreateNamedPipeW(PIPE_ACCESS_DUPLEX+FILE_FLAG_FIRST_PIPE_INSTANCE)",
        &format!(
            "squatter_create={} flagged_create={}",
            st(&sq),
            st(&flagged)
        ),
        "err:5 (ERROR_ACCESS_DENIED)",
        verdict(st(&sq) == "ok" && st(&flagged) == "err:5"),
    );
    let plain = me.cmd(&format!("create {name} 255 0"));
    check(
        "P1b",
        "control: the identical call without the flag",
        "CreateNamedPipeW(PIPE_ACCESS_DUPLEX)",
        &format!("create={}", st(&plain)),
        "ok (the hazard D2 closes)",
        verdict(st(&plain) == "ok"),
    );
    check(
        "P1c",
        "cross-user squatter",
        "squatter under runas /user:<second account>",
        "not run",
        "O2 not run (owner decision 2026-10-31); PORT_PLAN acceptance check step 4",
        Status::NotEvaluated,
    );
    drop(squatter);

    let name = pipe_name("p1d");
    let mut squatter = Agent::spawn("squatter");
    let sq = squatter.cmd(&format!("create {name} 1 0"));
    let mut client = Agent::spawn("client");
    let opened = client.cmd(&format!("open {name}"));
    let connected = squatter.cmd(&format!("connect {}", kv(&sq, "h")));
    let flagged = me.cmd(&format!("create {name} 1 1"));
    check(
        "P1d",
        "squatter at its instance limit (nMaxInstances 1, connected)",
        "CreateNamedPipeW(PIPE_ACCESS_DUPLEX+FILE_FLAG_FIRST_PIPE_INSTANCE nMaxInstances=1)",
        &format!(
            "squatter_create={} client_open={} squatter_connect={} flagged_create={}",
            st(&sq),
            st(&opened),
            st(&connected),
            st(&flagged)
        ),
        "recorded: 5 or 231 (ERROR_PIPE_BUSY)",
        Status::Recorded,
    );
}

// ---------------------------------------------------------------- P2: who the server is

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn p2() {
    let mut me = Local::default();
    let name = pipe_name("p2");
    let server = me.cmd(&format!("create {name} 255 1"));
    let mut client = Agent::spawn("client");
    let opened = client.cmd(&format!("open {name}"));
    let ch = kv(&opened, "h").to_string();
    let id = client.cmd(&format!("serverid {ch}"));
    let own = client.cmd("whoami");
    let probe_pid = std::process::id().to_string();
    check(
        "P2a",
        "server identity from a client handle",
        "GetNamedPipeServerProcessId+OpenProcess+OpenProcessToken+GetTokenInformation(TokenUser)",
        &format!(
            "server_pid={} probe_pid={probe_pid} server_user={} client_user={}",
            kv(&id, "pid"),
            kv(&id, "token_user"),
            kv(&own, "token_user")
        ),
        "pid = the server pid; SID = the client's own TokenUser SID",
        verdict(
            st(&id) == "ok"
                && kv(&id, "pid") == probe_pid
                && kv(&id, "token_user") == kv(&own, "token_user"),
        ),
    );
    let owner = client.cmd(&format!("owner {ch}"));
    let owner_got = format!(
        "owner={} type={} server_token_owner={} server_token_user={} owner_is_user={}",
        kv(&owner, "owner"),
        kv(&owner, "type"),
        kv(&id, "token_owner"),
        kv(&id, "token_user"),
        yes(kv(&owner, "owner") == kv(&id, "token_user"))
    );
    let owner_ok = st(&owner) == "ok" && kv(&owner, "owner") == kv(&id, "token_owner");
    check(
        "P2b",
        "pipe owner through the client handle",
        "GetSecurityInfo(client handle, OWNER_SECURITY_INFORMATION)",
        &owner_got,
        "owner SID = the server token's TokenOwner SID",
        verdict(owner_ok),
    );
    if elevated(own_token()) {
        check(
            "P2c",
            "pipe owner, server elevated: default descriptor",
            "GetSecurityInfo(client handle, OWNER_SECURITY_INFORMATION)",
            &owner_got,
            "owner = the elevated token's TokenOwner SID",
            verdict(owner_ok),
        );
        let mine = me.cmd("whoami");
        let user = kv(&mine, "token_user");
        let sddl = format!("O:{user}D:P(A;;GA;;;{user})(A;;GA;;;SY)(A;;GA;;;BA)");
        let name = pipe_name("p2c");
        let made = me.cmd(&format!("createsd {name} 255 1 {sddl}"));
        let opened = client.cmd(&format!("open {name}"));
        let owner = client.cmd(&format!("owner {}", kv(&opened, "h")));
        check(
            "P2c",
            "pipe owner, server elevated: explicit O:<user> descriptor",
            "ConvertStringSecurityDescriptorToSecurityDescriptorW+CreateNamedPipeW+GetSecurityInfo",
            &format!(
                "create={} open={} owner={} user={user}",
                st(&made),
                st(&opened),
                kv(&owner, "owner")
            ),
            "owner = the O: SID",
            verdict(st(&owner) == "ok" && kv(&owner, "owner") == user),
        );
    } else {
        check(
            "P2c",
            "pipe owner, server elevated",
            "the probe run elevated (owner step O1 (i))",
            "TokenElevation=0",
            "NOT EVALUATED unless O1 ran",
            Status::NotEvaluated,
        );
    }
    let sh = kv(&server, "h").to_string();
    let connected = me.cmd(&format!("connect {sh}"));
    let cp = me.cmd(&format!("clientpid {sh}"));
    check(
        "P2d",
        "client pid on the server side (D7 attribution)",
        "GetNamedPipeClientProcessId(server handle)",
        &format!(
            "server_connect={} client_pid={} client_process={}",
            st(&connected),
            kv(&cp, "pid"),
            kv(&own, "pid")
        ),
        "client_pid = the client process pid",
        verdict(st(&cp) == "ok" && kv(&cp, "pid") == kv(&own, "pid")),
    );
    for (id, pairing) in [
        ("P2e", "elevated server, non-elevated client (O1 (ii))"),
        ("P2f", "non-elevated server, elevated client (O1 (iii))"),
    ] {
        check(
            id,
            pairing,
            "pair-server and pair-client",
            "see the O1 pair files",
            "NOT EVALUATED unless O1 ran; there RECORDED per call",
            Status::NotEvaluated,
        );
    }
}

/// One side of an O1 pairing; the id follows the elevations (P2e: server elevated, client not).
fn pair(args: &[String], server: bool) {
    set_out(arg(args, "--out"));
    header();
    let name = format!(r"\\.\pipe\{}", arg(args, "--name").expect("--name"));
    let el = elevated(own_token());
    let id = if server == el { "P2e" } else { "P2f" };
    let mut me = Local::default();
    if server {
        // Sized for the UAC prompt of the other side, not measured.
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(120));
            out("pair-server: no client within 120 s");
            std::process::exit(1)
        });
        let made = me.cmd(&format!("create {name} 255 1"));
        let h = kv(&made, "h").to_string();
        let ok = st(&made) == "ok";
        let connected = if ok {
            me.cmd(&format!("connect {h}"))
        } else {
            "err not-created".into()
        };
        let cp = if ok {
            me.cmd(&format!("clientpid {h}"))
        } else {
            "err not-created".into()
        };
        check(
            id,
            "pair-server side: create, connect and client attribution",
            "CreateNamedPipeW(+FIRST_PIPE_INSTANCE)+ConnectNamedPipe+GetNamedPipeClientProcessId",
            &format!(
                "elevated={} create={} connect={} clientpid={} client_pid={}",
                el as u8,
                st(&made),
                st(&connected),
                st(&cp),
                kv(&cp, "pid")
            ),
            "recorded",
            Status::Recorded,
        );
        if ok {
            me.cmd(&format!("read {h}"));
        }
    } else {
        let t = Instant::now();
        let opened = loop {
            let r = me.cmd(&format!("open {name}"));
            if st(&r) == "ok" || t.elapsed() > Duration::from_secs(60) {
                break r;
            }
            thread::sleep(Duration::from_millis(100));
        };
        let h = kv(&opened, "h").to_string();
        let (sid, owner) = if st(&opened) == "ok" {
            (
                me.cmd(&format!("serverid {h}")),
                me.cmd(&format!("owner {h}")),
            )
        } else {
            ("err not-open".into(), "err not-open".into())
        };
        let mine = me.cmd("whoami");
        check(
            id,
            "pair-client side: server identity and pipe owner",
            "CreateFileW+GetNamedPipeServerProcessId+OpenProcess+OpenProcessToken+GetTokenInformation+GetSecurityInfo",
            &format!(
                "elevated={} open={} serverid={} step={} server_pid={} server_user={} server_owner={} server_elevated={} same_user={} pipe_owner_call={} pipe_owner={} type={}",
                el as u8,
                st(&opened),
                st(&sid),
                kv(&sid, "step"),
                kv(&sid, "pid"),
                kv(&sid, "token_user"),
                kv(&sid, "token_owner"),
                kv(&sid, "elevated"),
                yes(kv(&sid, "token_user") == kv(&mine, "token_user")),
                st(&owner),
                kv(&owner, "owner"),
                kv(&owner, "type")
            ),
            "recorded",
            Status::Recorded,
        );
    }
}

// ---------------------------------------------------------------- P4: busy, gone, killed

fn p4() {
    let mut me = Local::default();
    let name = pipe_name("p4a");
    let opened = me.cmd(&format!("open {name}"));
    let waited = me.cmd(&format!("wait {name} 500"));
    check(
        "P4a",
        "no pipe",
        "CreateFileW(OPEN_EXISTING)+WaitNamedPipeW(500)",
        &format!(
            "open={} wait={} wait_ms={}",
            st(&opened),
            st(&waited),
            ms(&waited)
        ),
        "open err:2 (ERROR_FILE_NOT_FOUND); wait returns zero at once (under 100 ms)",
        verdict(st(&opened) == "err:2" && waited.starts_with("err") && ms(&waited) < 100),
    );

    let name = pipe_name("p4b");
    let server = me.cmd(&format!("create {name} 255 1"));
    let mut c1 = Agent::spawn("client");
    let c1_open = c1.cmd(&format!("open {name}"));
    let connected = me.cmd(&format!("connect {}", kv(&server, "h")));
    let mut c2 = Agent::spawn("client");
    let c2_open = c2.cmd(&format!("open {name}"));
    let c2_wait = c2.cmd(&format!("wait {name} 500"));
    check(
        "P4b",
        "all instances busy",
        "CreateFileW+WaitNamedPipeW(500)",
        &format!(
            "client1_open={} server_connect={} client2_open={} client2_wait={} wait_ms={}",
            st(&c1_open),
            st(&connected),
            st(&c2_open),
            st(&c2_wait),
            ms(&c2_wait)
        ),
        "client2 open err:231 (ERROR_PIPE_BUSY); wait err:121 (ERROR_SEM_TIMEOUT) after ~500 ms",
        verdict(
            st(&c1_open) == "ok"
                && st(&c2_open) == "err:231"
                && st(&c2_wait) == "err:121"
                && (400..2000).contains(&ms(&c2_wait)),
        ),
    );

    let c1_close = c1.cmd(&format!("close {}", kv(&c1_open, "h")));
    let gap = c2.cmd(&format!("open {name}"));
    let second = me.cmd(&format!("create {name} 255 0"));
    let listening = (st(&second) == "ok").then(|| me.listen(kv(&second, "h")));
    thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    let after = c2.cmd(&format!("open {name}"));
    let open_ms = t.elapsed().as_millis();
    let connect = listening.map_or("not run".into(), |j| st(&j.join().unwrap()));
    check(
        "P4c",
        "window with no pending instance (client 1 closed, server not yet disconnected)",
        "CreateFileW; then CreateNamedPipeW(second instance)+ConnectNamedPipe",
        &format!(
            "client1_close={} client2_open={} second_create={} client2_open_after={} open_ms={open_ms} server_connect={connect}",
            st(&c1_close),
            st(&gap),
            st(&second),
            st(&after)
        ),
        "recorded: the raw code (231 expected); client 2 then connects at once",
        Status::Recorded,
    );

    let name = pipe_name("p4d");
    let server = me.cmd(&format!("create {name} 255 1"));
    let h = kv(&server, "h").to_string();
    let mut c = Agent::spawn("client");
    let opened = c.cmd(&format!("open {name}"));
    let connected = me.cmd(&format!("connect {h}"));
    let wrote = me.cmd(&format!("write {h} x"));
    let read = c.cmd(&format!("read {}", kv(&opened, "h")));
    check(
        "P4d",
        "client connects before ConnectNamedPipe is called",
        "ConnectNamedPipe after the client's CreateFileW",
        &format!(
            "client_open={} server_connect={} server_write={} client_read={} data={}",
            st(&opened),
            st(&connected),
            st(&wrote),
            st(&read),
            kv(&read, "data")
        ),
        "connect err:535 (ERROR_PIPE_CONNECTED); the client is served",
        verdict(st(&opened) == "ok" && st(&connected) == "err:535" && kv(&read, "data") == "x"),
    );

    let name = pipe_name("p4e");
    let mut srv = Agent::spawn("server");
    let created = srv.cmd(&format!("create {name} 255 1"));
    srv.kill();
    let opened = me.cmd(&format!("open {name}"));
    let waited = me.cmd(&format!("wait {name} 500"));
    let relaunch = me.cmd(&format!("create {name} 255 1"));
    check(
        "P4e",
        "server killed, no client connected",
        "TerminateProcess(server); CreateFileW; WaitNamedPipeW(500); flagged CreateNamedPipeW",
        &format!(
            "server_create={} open={} wait={} wait_ms={} relaunch_create={}",
            st(&created),
            st(&opened),
            st(&waited),
            ms(&waited),
            st(&relaunch)
        ),
        "open err:2; wait returns zero at once (under 100 ms); the new flagged create ok",
        verdict(
            st(&created) == "ok"
                && st(&opened) == "err:2"
                && waited.starts_with("err")
                && ms(&waited) < 100
                && st(&relaunch) == "ok",
        ),
    );

    let name = pipe_name("p4f");
    let mut srv = Agent::spawn("server");
    let created = srv.cmd(&format!("create {name} 255 1"));
    let mut c1 = Agent::spawn("client");
    let c1_open = c1.cmd(&format!("open {name}"));
    let connected = srv.cmd(&format!("connect {}", kv(&created, "h")));
    srv.kill();
    let relaunch = me.cmd(&format!("create {name} 255 1"));
    let mut c2 = Agent::spawn("client");
    let c2_open = c2.cmd(&format!("open {name}"));
    let h1 = kv(&c1_open, "h").to_string();
    let read = c1.cmd(&format!("read {h1}"));
    let write = c1.cmd(&format!("write {h1} x"));
    let c1_close = c1.cmd(&format!("close {h1}"));
    // Undo what the first try made, so the retry sees only what client 1's close changed.
    if st(&c2_open) == "ok" {
        c2.cmd(&format!("close {}", kv(&c2_open, "h")));
    }
    if st(&relaunch) == "ok" {
        me.cmd(&format!("close {}", kv(&relaunch, "h")));
    }
    let retry = me.cmd(&format!("create {name} 255 1"));
    check(
        "P4f",
        "server killed while client 1 holds a connected handle",
        "TerminateProcess(server); flagged CreateNamedPipeW; CreateFileW; ReadFile; WriteFile; retry after close",
        &format!(
            "server_create={} client1_open={} server_connect={} relaunch_create={} client2_open={} client1_read={} client1_write={} client1_close={} retry_create={}",
            st(&created),
            st(&c1_open),
            st(&connected),
            st(&relaunch),
            st(&c2_open),
            st(&read),
            st(&write),
            st(&c1_close),
            st(&retry)
        ),
        "recorded: each raw code (109 or 233 for client 1 expected; the retry ok)",
        Status::Recorded,
    );
}
