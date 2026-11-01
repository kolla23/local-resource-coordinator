//! Decision-gate probe for ADR 1 (named pipes, pid-file locks); not part of qex.
//! Usage: `named-pipe-probe [--out FILE]`; the other subcommands are its child roles.

use std::ffi::c_void;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, ptr};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::*;

const WINDOWS_SYS: &str = "0.61.2";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("squatter" | "server" | "client" | "holder" | "reader" | "deleter") => agent(),
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

// ---------------------------------------------------------------- header

fn run_all(args: &[String]) {
    set_out(arg(args, "--out"));
    header();
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
            _ => "err unknown-command".into(),
        }
    }
}
