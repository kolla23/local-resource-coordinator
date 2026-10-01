//! Decision-gate experiment: run N build+test workloads on native Windows,
//! either all at once (`parallel`) or one after another (`serial`), and record
//! what the machine went through. Standalone; not part of qex.
//!
//!   parallel-workload --mode parallel|serial --out <dir> --label <name> \
//!       --cmd "<command line>" [--temp-root <dir>] <workdir>...
//!
//! With `--temp-root`, workload i gets its own fresh `TEMP`/`TMP` at
//! `<dir>/<label>-w<i>`, so test harnesses can't collide in a shared `%TEMP%`.
//!
//! Each workload runs in its own Job Object, so its whole process tree (cargo,
//! rustc, link.exe, test binaries) is measured: the kernel's peak job commit.
//! A sampler records system commit and available RAM every 250 ms, `typeperf`
//! records paging, and a 100 ms timer measures how late the machine wakes it.

use std::ffi::c_void;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, ptr};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::Threading::*;

const MIB: f64 = (1u64 << 20) as f64;

struct Job(HANDLE);
// The handle is only used through thread-safe kernel calls.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let get = |n: &str| {
        args.iter()
            .position(|a| a == n)
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| panic!("missing {n}"))
    };
    let mode = get("--mode");
    let out = PathBuf::from(get("--out"));
    let label = get("--label");
    let cmd = get("--cmd");
    let temp_root = args
        .iter()
        .position(|a| a == "--temp-root")
        .map(|i| PathBuf::from(&args[i + 1]));
    let dirs: Vec<PathBuf> = {
        let mut skip = false;
        let mut v = Vec::new();
        for a in &args {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                v.push(PathBuf::from(a));
            }
        }
        v
    };
    assert!(
        mode == "parallel" || mode == "serial",
        "--mode parallel|serial"
    );
    fs::create_dir_all(&out).unwrap();
    run(&mode, &out, &label, &cmd, temp_root.as_deref(), &dirs);
}

fn perf() -> PERFORMANCE_INFORMATION {
    unsafe {
        let mut p: PERFORMANCE_INFORMATION = std::mem::zeroed();
        p.cb = size_of::<PERFORMANCE_INFORMATION>() as u32;
        assert!(GetPerformanceInfo(&mut p, p.cb) != 0);
        p
    }
}
fn commit_mib(p: &PERFORMANCE_INFORMATION) -> f64 {
    (p.CommitTotal * p.PageSize) as f64 / MIB
}
fn avail_mib(p: &PERFORMANCE_INFORMATION) -> f64 {
    (p.PhysicalAvailable * p.PageSize) as f64 / MIB
}
fn limit_mib(p: &PERFORMANCE_INFORMATION) -> f64 {
    (p.CommitLimit * p.PageSize) as f64 / MIB
}
fn total_mib(p: &PERFORMANCE_INFORMATION) -> f64 {
    (p.PhysicalTotal * p.PageSize) as f64 / MIB
}

fn new_job() -> Job {
    unsafe {
        let job = CreateJobObjectW(ptr::null(), ptr::null());
        assert!(!job.is_null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        assert!(
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of_val(&info) as u32,
            ) != 0
        );
        Job(job)
    }
}
fn job_info<T>(job: &Job, class: JOBOBJECTINFOCLASS) -> T {
    unsafe {
        let mut v: T = std::mem::zeroed();
        assert!(
            QueryInformationJobObject(
                job.0,
                class,
                &mut v as *mut T as *mut c_void,
                size_of::<T>() as u32,
                ptr::null_mut(),
            ) != 0
        );
        v
    }
}

/// One workload: started suspended inside its job, then resumed.
struct Run {
    dir: PathBuf,
    job: Job,
    process: HANDLE,
    started: Instant,
    ended: Option<Duration>,
    exit: Option<u32>,
}

fn start(job: Job, dir: &Path, cmd: &str, log: &Path) -> Run {
    unsafe {
        // Output goes to a file through cmd.exe redirection, so the child needs
        // no inherited pipes from us.
        // Parentheses so the redirection covers every command in a `&&` chain.
        let line = format!("cmd.exe /d /s /c \"({cmd}) > \"{}\" 2>&1\"", log.display());
        let mut wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let dirw: Vec<u16> = dir
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain([0])
            .collect();
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            ptr::null(),
            wide.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED,
            ptr::null(),
            dirw.as_ptr(),
            &si,
            &mut pi,
        );
        assert!(
            ok != 0,
            "CreateProcessW: {}",
            std::io::Error::last_os_error()
        );
        assert!(AssignProcessToJobObject(job.0, pi.hProcess) != 0);
        let started = Instant::now();
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        Run {
            dir: dir.to_path_buf(),
            job,
            process: pi.hProcess,
            started,
            ended: None,
            exit: None,
        }
    }
}

fn poll(r: &mut Run) {
    if r.ended.is_none() && unsafe { WaitForSingleObject(r.process, 0) } == WAIT_OBJECT_0 {
        r.ended = Some(r.started.elapsed());
        let mut code = 0;
        unsafe { GetExitCodeProcess(r.process, &mut code) };
        r.exit = Some(code);
    }
}

fn run(mode: &str, out: &Path, label: &str, cmd: &str, temp_root: Option<&Path>, dirs: &[PathBuf]) {
    let stop = Arc::new(AtomicBool::new(false));

    // Paging counters, once a second, for the whole run.
    let typeperf_csv = out.join(format!("{label}-typeperf.csv"));
    fs::remove_file(&typeperf_csv).ok();
    let mut typeperf: Child = Command::new("typeperf")
        .args([
            r"\Memory\Pages Input/sec",
            r"\Memory\Pages/sec",
            r"\Paging File(_Total)\% Usage",
            r"\Memory\Available MBytes",
            "-si",
            "1",
            "-f",
            "CSV",
            "-y",
            "-o",
        ])
        .arg(&typeperf_csv)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("typeperf");

    // Responsiveness: how late does a 100 ms sleep wake up?
    let stop_r = stop.clone();
    let resp = thread::spawn(move || {
        let (mut worst, mut over_1s, mut over_2s, mut n) = (Duration::ZERO, 0u32, 0u32, 0u32);
        while !stop_r.load(Ordering::Relaxed) {
            let t = Instant::now();
            thread::sleep(Duration::from_millis(100));
            let late = t.elapsed().saturating_sub(Duration::from_millis(100));
            worst = worst.max(late);
            over_1s += (late > Duration::from_secs(1)) as u32;
            over_2s += (late > Duration::from_secs(2)) as u32;
            n += 1;
        }
        (worst, over_1s, over_2s, n)
    });

    let idle = perf();
    thread::sleep(Duration::from_secs(1));
    let t0 = Instant::now();
    let mut series = String::from("t_s,commit_mib,avail_mib,active_procs\n");
    let (mut max_commit, mut min_avail) = (0f64, f64::MAX);
    let mut runs: Vec<Run> = Vec::new();
    let mut pending: Vec<&PathBuf> = dirs.iter().rev().collect();
    // Safety guard (SPEC §21.4): stop every workload if the machine stalls for
    // more than 10 s, or if system commit comes within 2 GiB of the limit.
    let mut aborted: Option<String> = None;
    let mut last_tick = Instant::now();

    loop {
        let gap = last_tick.elapsed();
        last_tick = Instant::now();
        let p = perf();
        if aborted.is_none() {
            if gap > Duration::from_secs(10) {
                aborted = Some(format!("sampler stalled {:.1} s", gap.as_secs_f64()));
            } else if limit_mib(&p) - commit_mib(&p) < 2048.0 {
                aborted = Some(format!(
                    "commit {:.0} MiB within 2 GiB of limit {:.0} MiB",
                    commit_mib(&p),
                    limit_mib(&p)
                ));
            }
            if let Some(why) = &aborted {
                eprintln!("ABORT: {why}");
                pending.clear();
                for r in &runs {
                    unsafe { TerminateJobObject(r.job.0, 1) };
                }
            }
        }
        // Start work: everything at once, or the next one when the last finished.
        let busy = runs.iter().any(|r| r.ended.is_none());
        while let Some(d) = pending.last() {
            if mode == "serial" && (busy || runs.iter().any(|r| r.ended.is_none())) {
                break;
            }
            let i = runs.len();
            let log = out.join(format!("{label}-w{i}.log"));
            let cmd = match temp_root {
                Some(root) => {
                    let tmp = root.join(format!("{label}-w{i}"));
                    fs::remove_dir_all(&tmp).ok();
                    fs::create_dir_all(&tmp).unwrap();
                    let t = tmp.display();
                    format!("set \"TEMP={t}\" && set \"TMP={t}\" && {cmd}")
                }
                None => cmd.to_string(),
            };
            runs.push(start(new_job(), d, &cmd, &log));
            pending.pop();
        }
        for r in &mut runs {
            poll(r);
        }
        let active: u32 = runs
            .iter()
            .map(|r| {
                job_info::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>(
                    &r.job,
                    JobObjectBasicAccountingInformation,
                )
                .ActiveProcesses
            })
            .sum();
        max_commit = max_commit.max(commit_mib(&p));
        min_avail = min_avail.min(avail_mib(&p));
        let _ = writeln!(
            series,
            "{:.2},{:.0},{:.0},{active}",
            t0.elapsed().as_secs_f64(),
            commit_mib(&p),
            avail_mib(&p)
        );
        if pending.is_empty() && runs.iter().all(|r| r.ended.is_some()) {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    let makespan = t0.elapsed();
    stop.store(true, Ordering::Relaxed);
    let (worst, over_1s, over_2s, n) = resp.join().unwrap();
    thread::sleep(Duration::from_secs(2));
    typeperf.kill().ok();
    typeperf.wait().ok();

    let mut s = String::new();
    let _ = writeln!(
        s,
        "label: {label}\nmode: {mode}\ncommand: {cmd}\nworkloads: {}",
        runs.len()
    );
    let _ = writeln!(
        s,
        "machine: physical {:.0} MiB, commit limit {:.0} MiB | idle before start: commit {:.0} MiB, available {:.0} MiB",
        total_mib(&idle),
        limit_mib(&idle),
        commit_mib(&idle),
        avail_mib(&idle)
    );
    let _ = writeln!(s, "makespan_s: {:.1}", makespan.as_secs_f64());
    let _ = writeln!(s, "aborted: {}", aborted.as_deref().unwrap_or("no"));
    let mut sum_peak = 0.0;
    for (i, r) in runs.iter().enumerate() {
        let ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
            job_info(&r.job, JobObjectExtendedLimitInformation);
        let acct: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
            job_info(&r.job, JobObjectBasicAccountingInformation);
        let peak = ext.PeakJobMemoryUsed as f64 / MIB;
        sum_peak += peak;
        let _ = writeln!(
            s,
            "w{i}: dir {} | exit {} | wall_s {:.1} | job_peak_commit_mib {:.0} | processes_total {} | still_running_after_root_exit {}",
            r.dir.display(),
            r.exit.map_or("?".into(), |c| c.to_string()),
            r.ended.unwrap_or_default().as_secs_f64(),
            peak,
            acct.TotalProcesses,
            acct.ActiveProcesses
        );
    }
    let _ = writeln!(
        s,
        "sum_of_job_peaks_mib: {sum_peak:.0}\nsystem_max_commit_mib: {max_commit:.0} (+{:.0} over idle)\nsystem_min_available_mib: {min_avail:.0}",
        max_commit - commit_mib(&idle)
    );
    let _ = writeln!(
        s,
        "responsiveness_100ms_timer: samples {n} | worst_late_ms {} | late_over_1s {over_1s} | late_over_2s {over_2s}",
        worst.as_millis()
    );
    let _ = writeln!(s, "typeperf_csv: {}", typeperf_csv.display());
    fs::write(out.join(format!("{label}-series.csv")), series).unwrap();
    fs::write(out.join(format!("{label}-summary.txt")), &s).unwrap();
    print!("{s}");

    for r in runs {
        unsafe {
            // Kill-on-close ends anything still running (e.g. a lingering PDB server).
            CloseHandle(r.process);
            CloseHandle(r.job.0);
        }
    }
}
