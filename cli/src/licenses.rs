use std::io::{self, Write};

use serde::Serialize;

#[derive(Serialize)]
struct Licenses {
    license: &'static str,
    license_text: &'static str,
    notice: &'static str,
    third_party_notices: &'static str,
}

const LICENSES: Licenses = Licenses {
    license: "Apache-2.0",
    license_text: include_str!("../../LICENSE"),
    notice: include_str!("../../NOTICE"),
    third_party_notices: include_str!("../../legal/third-party-rust.txt"),
};

pub fn run(json: bool) -> anyhow::Result<()> {
    let mut output = io::stdout().lock();
    match write_licenses(&mut output, json) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

fn write_licenses(mut output: impl Write, json: bool) -> io::Result<()> {
    if json {
        serde_json::to_writer(&mut output, &LICENSES)?;
        writeln!(output)?;
    } else {
        writeln!(output, "Scope — {}\n", LICENSES.license)?;
        writeln!(output, "{}", LICENSES.notice)?;
        writeln!(output, "{}", LICENSES.license_text)?;
        writeln!(output, "{}", LICENSES.third_party_notices)?;
    }
    Ok(())
}
