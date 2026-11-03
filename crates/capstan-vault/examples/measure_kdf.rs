//! Measure synthetic Vault unlocks without changing the production KDF policy.
#![allow(clippy::pedantic)]

use capstan_vault::{KdfParameters, SecretText, Vault};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use std::time::Instant;
use zeroize::Zeroizing;

#[derive(Serialize)]
struct Measurement {
    schema_version: u32,
    operation: &'static str,
    parameters: KdfParameters,
    warmup_count: u32,
    elapsed_ms: Vec<f64>,
}

fn measure() -> Result<Measurement, String> {
    let values = std::env::args()
        .skip(1)
        .map(|argument| argument.parse::<u32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "expected memory_kib iterations parallelism samples".to_owned())?;
    let [memory_kib, iterations, parallelism, samples] = values.as_slice() else {
        return Err("expected memory_kib iterations parallelism samples".into());
    };
    if !(3..=100).contains(samples) {
        return Err("samples must be between 3 and 100".into());
    }
    let parameters = KdfParameters {
        memory_kib: *memory_kib,
        iterations: *iterations,
        parallelism: *parallelism,
    };
    parameters
        .validate()
        .map_err(|error| error.code().to_owned())?;
    let mut random = Zeroizing::new([0u8; 32]);
    OsRng
        .try_fill_bytes(random.as_mut())
        .map_err(|_| "secure random generation failed".to_owned())?;
    let password = SecretText::new(
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    let directory = tempfile::tempdir().map_err(|_| "temporary directory failed".to_owned())?;
    let path = directory.path().join("synthetic.capstan");
    let vault =
        Vault::initialize(&path, &password, parameters).map_err(|error| error.code().to_owned())?;
    vault.lock().map_err(|error| error.code().to_owned())?;
    drop(vault);
    // Exclude initialization and one warmup unlock from the sample distribution.
    drop(Vault::unlock(&path, &password).map_err(|error| error.code().to_owned())?);
    let mut elapsed_ms = Vec::new();
    for _ in 0..*samples {
        let started = Instant::now();
        let vault = Vault::unlock(&path, &password).map_err(|error| error.code().to_owned())?;
        elapsed_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        vault.lock().map_err(|error| error.code().to_owned())?;
    }
    Ok(Measurement {
        schema_version: 1,
        operation: "synthetic_vault_unlock_including_file_io",
        parameters,
        warmup_count: 1,
        elapsed_ms,
    })
}

fn main() -> std::process::ExitCode {
    match measure().and_then(|measurement| {
        serde_json::to_string(&measurement).map_err(|_| "measurement encoding failed".to_owned())
    }) {
        Ok(json) => {
            println!("{json}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
