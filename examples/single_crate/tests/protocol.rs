use single_crate::{Acc, Gyro, ImuSample, Request, Response, SamplingRate, SensorConfig};

fn sample_response() -> Response {
    Response::Sample {
        sequence: 42,
        sample: ImuSample {
            timestamp_us: 1_000_000,
            gyro: Gyro {
                x: 1.0,
                y: -2.0,
                z: 3.5,
            },
            acc: Acc {
                x: 0.0,
                y: 9.81,
                z: -9.81,
            },
            temperature_c: Some(24.5),
        },
    }
}

#[test]
fn rust_protocol_roundtrips_without_allocation() {
    let mut buffer = [0; 128];
    for request in [
        Request::GetSample,
        Request::Configure(SensorConfig {
            sampling_rate: SamplingRate::Hz100,
            gyro_enabled: true,
            acc_enabled: true,
        }),
    ] {
        let wire = postcard::to_slice(&request, &mut buffer).unwrap();
        assert_eq!(postcard::from_bytes::<Request>(wire).unwrap(), request);
    }
    for response in [
        sample_response(),
        Response::Configured,
        Response::Error { code: 7 },
    ] {
        let wire = postcard::to_slice(&response, &mut buffer).unwrap();
        assert_eq!(postcard::from_bytes::<Response>(wire).unwrap(), response);
    }
}

#[cfg(feature = "c-bindings")]
#[test]
fn generated_c_sample_matches_rust() {
    use std::{path::Path, process::Command};

    let bindings = Path::new(single_crate::C_BINDINGS_DIR);
    for name in ["bindings.h", "bindings.c", "postcard.h", "postcard.c"] {
        assert!(
            bindings.join(name).is_file(),
            "missing generated file {name}"
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("sensor-roundtrip");
    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let output = Command::new(compiler)
        .args([
            "-std=c23",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pedantic",
            "-I",
        ])
        .arg(bindings)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("c/roundtrip.c"))
        .arg(bindings.join("bindings.c"))
        .arg(bindings.join("postcard.c"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("C example tests require a C23 compiler (set CC to its path)");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(executable).output().unwrap();
    assert!(
        output.status.success(),
        "C example exited with {}",
        output.status
    );
    let expected = sample_response();
    let mut buffer = [0; 128];
    assert_eq!(
        output.stdout,
        postcard::to_slice(&expected, &mut buffer).unwrap()
    );
    assert_eq!(
        postcard::from_bytes::<Response>(&output.stdout).unwrap(),
        expected
    );
}
