//! Decision-gate probe: can native Windows Job Objects own, measure and cancel a
//! whole process tree? Standalone experiment; not part of qex.
//!
//!   jobobject-probe                 run every scenario and print the results
//!   jobobject-probe worker ...      one node of a test process tree
//!   jobobject-probe owner ...       a stand-in coordinator (scenario 7)

use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{env, fs, ptr, thread};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    AllocConsole, CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent, GetConsoleWindow,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Threading::*;

const MB: usize = 1 << 20;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("worker") => worker(&args[1..]),
        Some("owner") => owner(&args[1..]),
        _ => run_all(),
    }
}

// ---------------------------------------------------------------- arguments

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}
fn num(args: &[String], name: &str) -> u64 {
    arg(args, name).map_or(0, |v| v.parse().unwrap())
}
fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
fn report(path: &Path, line: &str) {
    // One file per process (`<path>.<pid>`): no contention between processes.
    let mine = PathBuf::from(format!("{}.{}", path.display(), std::process::id()));
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(mine)
        .unwrap();
    f.write_all(format!("{line}\n").as_bytes()).unwrap();
    f.sync_all().ok();
}
fn exe() -> String {
    env::current_exe().unwrap().display().to_string()
}

// ---------------------------------------------------------------- worker

static REPORT: OnceLock<PathBuf> = OnceLock::new();

unsafe extern "system" fn on_ctrl(_event: u32) -> i32 {
    if let Some(p) = REPORT.get() {
        report(p, &format!("graceful {}", std::process::id()));
    }
    std::process::exit(0)
}

/// Allocates and touches memory, spawns children, holds, exits.
fn worker(args: &[String]) {
    let rep = PathBuf::from(arg(args, "--report").unwrap());
    REPORT.set(rep.clone()).unwrap();
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    report(
        &rep,
        &format!(
            "start {} depth {}",
            std::process::id(),
            num(args, "--depth")
        ),
    );

    let burst = num(args, "--burst-mb") as usize;
    if burst > 0 {
        let b = vec![1u8; burst * MB];
        std::hint::black_box(&b);
        thread::sleep(Duration::from_millis(num(args, "--burst-ms")));
    }
    let mem = vec![1u8; num(args, "--mb") as usize * MB];
    std::hint::black_box(&mem);

    let depth = num(args, "--depth");
    let mut kids = Vec::new();
    if depth > 0 {
        for _ in 0..num(args, "--children") {
            kids.push(
                Command::new(exe())
                    .args(["worker", "--report", rep.to_str().unwrap()])
                    .args(["--mb", &num(args, "--mb").to_string()])
                    .args(["--children", &num(args, "--children").to_string()])
                    .args(["--depth", &(depth - 1).to_string()])
                    .args(["--hold-ms", &num(args, "--hold-ms").to_string()])
                    .spawn()
                    .unwrap(),
            );
        }
    }
    let hold = [
        "worker",
        "--mb",
        "10",
        "--hold-ms",
        "60000",
        "--report",
        rep.to_str().unwrap(),
    ];
    if flag(args, "--breakaway") {
        use std::os::windows::process::CommandExt;
        match Command::new(exe())
            .args(hold)
            .creation_flags(CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
        {
            Ok(c) => report(&rep, &format!("breakaway allowed {}", c.id())),
            Err(e) => report(&rep, &format!("breakaway denied {e}")),
        }
    }
    if flag(args, "--nested") {
        let inner = new_job(false);
        let c = Command::new(exe()).args(hold).spawn().unwrap();
        use std::os::windows::io::AsRawHandle;
        let ok = unsafe { AssignProcessToJobObject(inner, c.as_raw_handle() as HANDLE) } != 0;
        report(&rep, &format!("nested {} assigned={ok}", c.id()));
    }
    if flag(args, "--wmi") {
        let cmdline = format!("{} {}", exe(), hold.join(" "));
        let ps = format!(
            "(Invoke-CimMethod -ClassName Win32_Process -MethodName Create \
             -Arguments @{{CommandLine='{cmdline}'}}).ProcessId"
        );
        let out = Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .unwrap();
        report(
            &rep,
            &format!("wmi {}", String::from_utf8_lossy(&out.stdout).trim()),
        );
    }
    thread::sleep(Duration::from_millis(num(args, "--hold-ms")));
    drop(kids);
}

// ---------------------------------------------------------------- job helpers

fn new_job(kill_on_close: bool) -> HANDLE {
    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        assert!(!job.is_null(), "CreateJobObjectW failed");
        if kill_on_close {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of_val(&info) as u32,
            );
            assert!(ok != 0, "SetInformationJobObject failed");
        }
        job
    }
}

/// Starts `cmdline` suspended, puts it in `job`, checks membership, then resumes.
/// Returns (process handle, pid, was it in the job before any user code ran).
fn launch_in_job(job: HANDLE, cmdline: &str) -> (HANDLE, u32, bool) {
    unsafe {
        let mut wide: Vec<u16> = cmdline.encode_utf16().chain([0]).collect();
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            ptr::null(),
            wide.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP,
            ptr::null(),
            ptr::null(),
            &si,
            &mut pi,
        );
        assert!(
            ok != 0,
            "CreateProcessW failed: {}",
            std::io::Error::last_os_error()
        );
        assert!(
            AssignProcessToJobObject(job, pi.hProcess) != 0,
            "AssignProcessToJobObject failed"
        );
        let mut in_job = 0;
        IsProcessInJob(pi.hProcess, job, &mut in_job);
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        (pi.hProcess, pi.dwProcessId, in_job != 0)
    }
}

fn query<T>(job: HANDLE, class: JOBOBJECTINFOCLASS) -> T {
    unsafe {
        let mut v: T = std::mem::zeroed();
        let ok = QueryInformationJobObject(
            job,
            class,
            &mut v as *mut T as *mut c_void,
            size_of::<T>() as u32,
            ptr::null_mut(),
        );
        assert!(ok != 0, "QueryInformationJobObject failed");
        v
    }
}
fn active(job: HANDLE) -> u32 {
    query::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>(job, JobObjectBasicAccountingInformation)
        .ActiveProcesses
}
fn total(job: HANDLE) -> u32 {
    query::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>(job, JobObjectBasicAccountingInformation)
        .TotalProcesses
}
fn peak_commit(job: HANDLE) -> usize {
    query::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(job, JobObjectExtendedLimitInformation)
        .PeakJobMemoryUsed
}
fn pids(job: HANDLE) -> Vec<u32> {
    // Header (two u32) then usize ids; room for 1024 processes.
    let mut buf = vec![0usize; 1 + 1024];
    unsafe {
        let ok = QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * size_of::<usize>()) as u32,
            ptr::null_mut(),
        );
        assert!(ok != 0, "process id list failed");
        let n = (buf[0] >> 32) as usize; // NumberOfProcessIdsInList
        buf[1..=n].iter().map(|&p| p as u32).collect()
    }
}
/// (working set, private commit) of one process, if it can still be opened.
fn mem_of(pid: u32) -> Option<(usize, usize)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = GetProcessMemoryInfo(h, &mut c as *mut _ as *mut _, c.cb);
        CloseHandle(h);
        (ok != 0).then_some((c.WorkingSetSize, c.PrivateUsage))
    }
}
fn alive(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        let r = WaitForSingleObject(h, 0);
        CloseHandle(h);
        r == WAIT_TIMEOUT
    }
}
fn kill_pid(pid: u32) {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}
fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> Option<Duration> {
    let t = Instant::now();
    while t.elapsed() < limit {
        if done() {
            return Some(t.elapsed());
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}
fn tree_size(children: u64, depth: u64) -> u32 {
    (0..=depth).map(|d| children.pow(d as u32) as u32).sum()
}
fn report_lines(path: &Path, prefix: &str) -> Vec<String> {
    let stem = format!("{}.", path.file_name().unwrap().to_string_lossy());
    let mut text = String::new();
    for e in fs::read_dir(path.parent().unwrap()).unwrap().flatten() {
        if e.file_name().to_string_lossy().starts_with(&stem) {
            text += &fs::read_to_string(e.path()).unwrap_or_default();
        }
    }
    text.lines()
        .filter(|l| l.starts_with(prefix))
        .map(|l| l[prefix.len()..].trim().to_string())
        .collect()
}
fn worker_cmd(rep: &Path, extra: &str) -> String {
    format!("{} worker --report {} {extra}", exe(), rep.display())
}
fn scratch(name: &str) -> PathBuf {
    let p = env::temp_dir().join(format!("jobprobe-{}-{name}.txt", std::process::id()));
    let stem = format!("{}.", p.file_name().unwrap().to_string_lossy());
    for e in fs::read_dir(env::temp_dir()).unwrap().flatten() {
        if e.file_name().to_string_lossy().starts_with(&stem) {
            fs::remove_file(e.path()).ok();
        }
    }
    p
}
fn mib(b: usize) -> String {
    format!("{:.1} MiB", b as f64 / MB as f64)
}

// ---------------------------------------------------------------- scenarios

fn run_all() {
    unsafe {
        if GetConsoleWindow().is_null() {
            AllocConsole(); // GenerateConsoleCtrlEvent needs a shared console.
        }
        let mut in_any = 0;
        IsProcessInJob(GetCurrentProcess(), ptr::null_mut(), &mut in_any);
        println!(
            "probe pid {} | probe itself already in a job: {}",
            std::process::id(),
            in_any != 0
        );
    }
    measure_tree();
    burst();
    cancel(true);
    cancel(false);
    breakaway();
    nested();
    owner_crash();
    wmi_escape();
}

/// Scenarios 1 + 2: contain from the first instruction, and measure the tree.
fn measure_tree() {
    let rep = scratch("tree");
    let job = new_job(true);
    let (h, _pid, in_job) = launch_in_job(
        job,
        &worker_cmd(&rep, "--mb 100 --children 2 --depth 2 --hold-ms 3000"),
    );
    let t = Instant::now();
    let (mut peak_ws, mut peak_priv, mut max_seen) = (0, 0, 0);
    while active(job) > 0 && t.elapsed() < Duration::from_secs(60) {
        let ps = pids(job);
        let m: Vec<_> = ps.iter().filter_map(|&p| mem_of(p)).collect();
        peak_ws = peak_ws.max(m.iter().map(|x| x.0).sum());
        peak_priv = peak_priv.max(m.iter().map(|x| x.1).sum());
        max_seen = max_seen.max(ps.len());
        thread::sleep(Duration::from_millis(100));
    }
    println!("\n[1] contain: in job before resume = {in_job}");
    println!(
        "[2] tree: expected {} processes x 100 MiB = 700 MiB allocated | total processes {} | max seen at once {} | wall {:.2}s",
        tree_size(2, 2),
        total(job),
        max_seen,
        t.elapsed().as_secs_f64()
    );
    println!(
        "    kernel peak job commit {} | sampled(100ms) peak sum working set {} | sampled peak sum private {}",
        mib(peak_commit(job)),
        mib(peak_ws),
        mib(peak_priv)
    );
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}

/// Scenario 3: a 400 MiB spike lasting 150 ms.
fn burst() {
    let rep = scratch("burst");
    let job = new_job(true);
    let (h, _, _) = launch_in_job(
        job,
        &worker_cmd(&rep, "--mb 0 --burst-mb 400 --burst-ms 150 --hold-ms 1500"),
    );
    let mut peak_priv = 0;
    while active(job) > 0 {
        let p: usize = pids(job)
            .iter()
            .filter_map(|&p| mem_of(p))
            .map(|x| x.1)
            .sum();
        peak_priv = peak_priv.max(p);
        thread::sleep(Duration::from_millis(250));
    }
    println!(
        "\n[3] burst 400 MiB for 150 ms: kernel peak job commit {} | sampled(250ms) peak private {}",
        mib(peak_commit(job)),
        mib(peak_priv)
    );
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}

/// Scenario 4: cancel a running tree; graceful (Ctrl-Break, 2 s grace) or hard.
fn cancel(graceful_first: bool) {
    let rep = scratch("cancel");
    let job = new_job(true);
    let (h, root, _) = launch_in_job(
        job,
        &worker_cmd(&rep, "--mb 20 --children 2 --depth 2 --hold-ms 60000"),
    );
    let want = tree_size(2, 2);
    wait_until(Duration::from_secs(30), || active(job) == want).expect("tree did not start");
    // Every worker has installed its Ctrl-Break handler (it reports "start" after that).
    wait_until(Duration::from_secs(30), || {
        report_lines(&rep, "start").len() == want as usize
    })
    .expect("workers did not report start");
    let before = pids(job);
    let t = Instant::now();
    let mut graceful_done = false;
    if graceful_first {
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, root) };
        graceful_done = wait_until(Duration::from_secs(2), || active(job) == 0).is_some();
    }
    if !graceful_done {
        unsafe { TerminateJobObject(job, 1) };
    }
    // Two clocks: the job's counter reaching zero, and every process object
    // actually signalled as exited (termination completes asynchronously).
    let counter_zero =
        wait_until(Duration::from_secs(10), || active(job) == 0).map(|_| t.elapsed());
    let all_exited = wait_until(Duration::from_secs(10), || {
        before.iter().all(|&p| !alive(p))
    })
    .map(|_| t.elapsed());
    let survivors: Vec<_> = before.iter().filter(|&&p| alive(p)).collect();
    let graceful = report_lines(&rep, "graceful");
    let not_graceful: Vec<_> = report_lines(&rep, "start")
        .into_iter()
        .filter(|l| !graceful.iter().any(|g| l.starts_with(&format!("{g} "))))
        .collect();
    println!(
        "\n[4] cancel ({}) of {} processes: graceful exits {} | not graceful (pid depth) {:?} | forced = {} | job counter zero after {:?} | all exited after {:?} | survivors after 10 s {:?}",
        if graceful_first {
            "Ctrl-Break, 2 s grace, then force"
        } else {
            "force only"
        },
        before.len(),
        graceful.len(),
        not_graceful,
        !graceful_done,
        counter_zero,
        all_exited,
        survivors
    );
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}

/// Scenario 5: a child asks to leave the job.
fn breakaway() {
    let rep = scratch("breakaway");
    let job = new_job(true);
    let (h, _, _) = launch_in_job(job, &worker_cmd(&rep, "--mb 0 --breakaway --hold-ms 3000"));
    wait_until(Duration::from_secs(10), || {
        !report_lines(&rep, "breakaway").is_empty()
    });
    let r = report_lines(&rep, "breakaway");
    println!("\n[5] breakaway attempt: {r:?}");
    unsafe { TerminateJobObject(job, 1) };
    // Clean up a child that left the job, if one did.
    for l in &r {
        if let Some(p) = l.strip_prefix("allowed ") {
            kill_pid(p.parse().unwrap());
        }
    }
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}

/// Scenario 6: a descendant placed in its own (nested) job.
fn nested() {
    let rep = scratch("nested");
    let job = new_job(true);
    let (h, _, _) = launch_in_job(job, &worker_cmd(&rep, "--mb 0 --nested --hold-ms 60000"));
    wait_until(Duration::from_secs(10), || {
        !report_lines(&rep, "nested").is_empty()
    });
    let line = report_lines(&rep, "nested").pop().unwrap_or_default();
    let pid: u32 = line
        .split_whitespace()
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let in_outer = pids(job).contains(&pid);
    unsafe { TerminateJobObject(job, 1) };
    let gone = wait_until(Duration::from_secs(5), || !alive(pid)).is_some();
    println!(
        "\n[6] nested job: {line} | still listed in outer job {in_outer} | killed by outer cancel {gone}"
    );
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}

/// Stand-in coordinator for scenario 7: owns a job, launches a tree, waits.
fn owner(args: &[String]) {
    let rep = PathBuf::from(arg(args, "--report").unwrap());
    let job = new_job(true);
    let tree = scratch("owner-tree");
    launch_in_job(
        job,
        &worker_cmd(&tree, "--mb 20 --children 2 --depth 2 --hold-ms 60000"),
    );
    wait_until(Duration::from_secs(30), || active(job) == tree_size(2, 2));
    let list: Vec<String> = pids(job).iter().map(u32::to_string).collect();
    report(&rep, &format!("pids {}", list.join(",")));
    thread::sleep(Duration::from_secs(120));
}

/// Scenario 7: the owner dies without cleaning up.
fn owner_crash() {
    let rep = scratch("owner");
    let mut o = Command::new(exe())
        .args(["owner", "--report", rep.to_str().unwrap()])
        .spawn()
        .unwrap();
    wait_until(Duration::from_secs(30), || {
        !report_lines(&rep, "pids").is_empty()
    })
    .expect("owner did not report");
    let tree: Vec<u32> = report_lines(&rep, "pids")[0]
        .split(',')
        .map(|p| p.parse().unwrap())
        .collect();
    o.kill().unwrap(); // TerminateProcess: no cleanup code runs in the owner.
    let t = Instant::now();
    o.wait().unwrap();
    let gone = wait_until(Duration::from_secs(10), || tree.iter().all(|&p| !alive(p)));
    let survivors: Vec<_> = tree.iter().filter(|&&p| alive(p)).collect();
    println!(
        "\n[7] owner killed: tree of {} processes stopped by kill-on-close after {:?} | survivors {:?}",
        tree.len(),
        gone.map(|_| t.elapsed()),
        survivors
    );
    for p in survivors {
        kill_pid(*p);
    }
}

/// Scenario 8: a child starts a process through WMI (outside the job's parentage).
fn wmi_escape() {
    let rep = scratch("wmi");
    let job = new_job(true);
    let (h, _, _) = launch_in_job(job, &worker_cmd(&rep, "--mb 0 --wmi --hold-ms 60000"));
    wait_until(Duration::from_secs(60), || {
        !report_lines(&rep, "wmi").is_empty()
    });
    let pid: u32 = report_lines(&rep, "wmi")
        .pop()
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);
    let in_job = pids(job).contains(&pid);
    unsafe { TerminateJobObject(job, 1) };
    thread::sleep(Duration::from_millis(500));
    let escaped = pid != 0 && alive(pid);
    println!(
        "\n[8] WMI-created process pid {pid}: in job {in_job} | still running after job cancel {escaped}"
    );
    if escaped {
        kill_pid(pid);
    }
    unsafe { CloseHandle(h) };
    unsafe { CloseHandle(job) };
}
