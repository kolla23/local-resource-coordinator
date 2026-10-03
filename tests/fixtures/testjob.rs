//! `testjob`: a portable job for the end-to-end tests.
//!
//! The tests used to run Unix programs as their jobs (`true`, `false`,
//! `sleep`, `echo`, `sh -c "..."`). None of them exists on Windows. This
//! program does the same work with the standard library only, so a test that
//! uses it runs on every platform the coordinator supports.
//!
//! The arguments are a list of steps, run in order:
//!
//!     exit N              stop with the exit code N
//!     sleep SECS          wait SECS seconds (a fraction is allowed)
//!     print TEXT          write TEXT and a new line to stdout
//!     print TEXT --stderr write it to stderr instead
//!     hold-mem MIB SECS   hold MIB mebibytes of touched memory for SECS seconds
//!     spin N              count to N in a busy loop
//!     count FILE          add 1 to the number in FILE (0 if there is no FILE)
//!     if-count-below N    run the steps up to the next `end` only when the
//!                         last `count` gave less than N; skip them otherwise
//!     end                 the end of an `if-count-below`
//!
//! After the last step the program exits 0. `sh -c "echo bad >&2; exit 9"` is
//! `testjob print bad --stderr exit 9`. A job that fails on its first attempt
//! only is `testjob count FILE if-count-below 2 exit 3 end`.
//!
//! It is built only with the `test-fixtures` feature, so `cargo install` never
//! installs it.

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("testjob: {message}");
            eprintln!(
                "usage: testjob [exit N | sleep SECS | print TEXT [--stderr] \
                 | hold-mem MIB SECS | spin N | count FILE \
                 | if-count-below N ... end]..."
            );
            ExitCode::from(2)
        }
    }
}

/// Runs the steps, and gives the exit code.
fn run(args: &[String]) -> Result<u8, String> {
    let mut rest = args;
    let mut count: u64 = 0;
    while let Some((step, tail)) = rest.split_first() {
        rest = tail;
        match step.as_str() {
            "exit" => {
                let code = take(&mut rest, "exit")?;
                return code
                    .parse::<u8>()
                    .map_err(|_| format!("exit needs a code from 0 to 255, not {code:?}"));
            }
            "sleep" => {
                let secs = seconds(take(&mut rest, "sleep")?)?;
                std::thread::sleep(secs);
            }
            "print" => {
                let text = take(&mut rest, "print")?;
                if rest.first().map(String::as_str) == Some("--stderr") {
                    rest = &rest[1..];
                    eprintln!("{text}");
                } else {
                    println!("{text}");
                    // A job's output goes to a file. Write it now, so a reader
                    // sees it before a later step sleeps.
                    let _ = std::io::stdout().flush();
                }
            }
            "hold-mem" => {
                let mib = take(&mut rest, "hold-mem")?;
                let mib: usize = mib
                    .parse()
                    .map_err(|_| format!("hold-mem needs a size in MiB, not {mib:?}"))?;
                let secs = seconds(take(&mut rest, "hold-mem")?)?;
                // Touch one byte in every page, so the memory is really in use
                // and not only promised.
                let mut block = vec![0u8; mib * 1024 * 1024];
                for page in block.iter_mut().step_by(4096) {
                    *page = 1;
                }
                std::thread::sleep(secs);
                std::hint::black_box(&block);
            }
            "spin" => {
                let n = take(&mut rest, "spin")?;
                let n: u64 = n
                    .parse()
                    .map_err(|_| format!("spin needs a count, not {n:?}"))?;
                let mut i = 0u64;
                while std::hint::black_box(i) < n {
                    i += 1;
                }
            }
            "count" => {
                let file = take(&mut rest, "count")?;
                let before = match std::fs::read_to_string(file) {
                    Ok(text) => text
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| format!("count found no number in {file:?}"))?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(e) => return Err(format!("count could not read {file:?}: {e}")),
                };
                count = before + 1;
                std::fs::write(file, format!("{count}\n"))
                    .map_err(|e| format!("count could not write {file:?}: {e}"))?;
            }
            "if-count-below" => {
                let n = take(&mut rest, "if-count-below")?;
                let n: u64 = n
                    .parse()
                    .map_err(|_| format!("if-count-below needs a number, not {n:?}"))?;
                let end = rest
                    .iter()
                    .position(|s| s == "end")
                    .ok_or("if-count-below needs an `end`")?;
                if count >= n {
                    rest = &rest[end + 1..];
                }
            }
            "end" => {}
            other => return Err(format!("unknown step {other:?}")),
        }
    }
    Ok(0)
}

/// Takes the value of a step.
fn take<'a>(rest: &mut &'a [String], step: &str) -> Result<&'a str, String> {
    let (value, tail) = rest
        .split_first()
        .ok_or_else(|| format!("{step} needs a value"))?;
    *rest = tail;
    Ok(value.as_str())
}

fn seconds(text: &str) -> Result<Duration, String> {
    text.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(|| format!("a time in seconds, not {text:?}"))
}
