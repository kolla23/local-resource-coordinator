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
//!
//! After the last step the program exits 0. `sh -c "echo bad >&2; exit 9"` is
//! `testjob print bad --stderr exit 9`.
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
                 | hold-mem MIB SECS]..."
            );
            ExitCode::from(2)
        }
    }
}

/// Runs the steps, and gives the exit code.
fn run(args: &[String]) -> Result<u8, String> {
    let mut rest = args;
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
