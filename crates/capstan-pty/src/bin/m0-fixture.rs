//! Fixed synthetic fixture; never executes commands supplied by the `WebView`.
#![deny(unsafe_code)]
use std::{
    io::{self, BufRead, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

fn main() -> io::Result<()> {
    let bulk = match std::env::args().nth(1).as_deref() {
        Some("--bulk") => true,
        Some("--idle") => false,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid fixture mode",
            ))
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = stop.clone();
    let output = thread::spawn(move || -> io::Result<()> {
        if bulk {
            let mut block = [b'a'; 16 * 1024];
            for (index, byte) in block.iter_mut().enumerate() {
                *byte = if index % 80 == 79 {
                    b'\n'
                } else {
                    b"abcdefghijklmnopqrstuvwxyz"[index % 26]
                };
            }
            for sequence in 0..(capstan_pty::benchmark::BULK_BYTES / block.len() as u64) {
                if writer_stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                block[..8].copy_from_slice(format!("{sequence:08x}").as_bytes());
                let mut stdout = io::stdout().lock();
                stdout.write_all(&block)?;
                stdout.flush()?;
                // Release the stdout lock between blocks so probes can respond.
            }
        }
        writeln!(io::stdout().lock(), "\r\nCAPSTAN_M0_END\r")?;
        Ok(())
    });
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        // The bound includes the delimiter. A hostile stream cannot allocate an
        // unbounded line even though this process only receives synthetic input.
        if Read::by_ref(&mut reader).take(64).read_line(&mut line)? == 0 || line.len() >= 64 {
            break;
        }
        let command = line.trim();
        if command == "exit" {
            break;
        }
        if let Some(sequence) = command.strip_prefix("probe:") {
            if sequence.len() == 6 && sequence.bytes().all(|byte| byte.is_ascii_digit()) {
                writeln!(io::stdout().lock(), "\r\nCAPSTAN_M0_PROBE:{sequence}\r")?;
            }
        }
    }
    stop.store(true, Ordering::Release);
    output
        .join()
        .map_err(|_| io::Error::other("fixture task failed"))??;
    Ok(())
}
