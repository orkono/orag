//! Windows installer smoke test helper (Task 47): starts a program in its own
//! console process group, waits until `GET <path>` on `<addr>` answers 200 and
//! prints that body, then sends it Ctrl-Break and waits up to 15 s for it to
//! exit. Exits with the program's exit code, or 124 when it does not stop.
//!
//! Usage: `ctrl_break <addr> <path> -- <program> [args...]`

#[cfg(windows)]
fn main() {
    std::process::exit(windows::run(std::env::args().skip(1).collect()));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ctrl_break is a Windows-only helper");
    std::process::exit(2);
}

#[cfg(windows)]
mod windows {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
    use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

    const READY_LIMIT: Duration = Duration::from_secs(120);
    const STOP_LIMIT: Duration = Duration::from_secs(15);

    pub fn run(args: Vec<String>) -> i32 {
        let Some(split) = args.iter().position(|a| a == "--") else {
            eprintln!("usage: ctrl_break <addr> <path> -- <program> [args...]");
            return 2;
        };
        let (addr, path) = (&args[0], &args[1]);
        let Some((program, rest)) = args[split + 1..].split_first() else {
            eprintln!("no program given");
            return 2;
        };
        let mut child = match Command::new(program)
            .args(rest)
            .creation_flags(CREATE_NEW_PROCESS_GROUP)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("cannot start {program}: {e}");
                return 2;
            }
        };
        match wait_ready(&mut child, addr, path) {
            Ok(body) => println!("{body}"),
            Err(e) => {
                eprintln!("{e}");
                let _ = child.kill();
                return 1;
            }
        }
        // SAFETY: plain Win32 call with the child's process group id (its pid).
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) } == 0 {
            eprintln!(
                "GenerateConsoleCtrlEvent failed: {}",
                std::io::Error::last_os_error()
            );
            let _ = child.kill();
            return 1;
        }
        let deadline = Instant::now() + STOP_LIMIT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("stopped after Ctrl-Break: {status}");
                    return status.code().unwrap_or(1);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Ok(None) => {
                    eprintln!(
                        "did not stop within {} s of Ctrl-Break",
                        STOP_LIMIT.as_secs()
                    );
                    let _ = child.kill();
                    return 124;
                }
                Err(e) => {
                    eprintln!("waiting for the program: {e}");
                    return 1;
                }
            }
        }
    }

    fn wait_ready(child: &mut Child, addr: &str, path: &str) -> Result<String, String> {
        let deadline = Instant::now() + READY_LIMIT;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("the program exited before it was ready: {status}"));
            }
            if let Some(body) = get(addr, path) {
                return Ok(body);
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "no 200 from {addr}{path} within {} s",
                    READY_LIMIT.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// The body of a 200 answer, or None while the server is not there yet.
    fn get(addr: &str, path: &str) -> Option<String> {
        let mut stream = TcpStream::connect(addr).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        let (head, body) = response.split_once("\r\n\r\n")?;
        head.starts_with("HTTP/1.1 200").then(|| body.to_string())
    }
}
