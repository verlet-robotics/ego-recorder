mod convert;
mod format;
mod h264_annex_b;
mod mp4_mux;

use std::path::PathBuf;

fn usage(program: &str) -> String {
    format!(
        "Usage:\n  {program} <input.egorec> <output.mp4>\n  {program} mp4 <input.egorec> <output.mp4>\n  {program} init <input.egorec> <output.mp4> <timescale> <sample_delta>\n  {program} segment <input.egorec> <output.m4s> <timescale> <sample_delta> <sequence_number> <base_decode_time>"
    )
}

fn parse_u32(value: &str, field: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .map_err(|err| format!("invalid {field}: {err}"))
}

fn parse_u64(value: &str, field: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|err| format!("invalid {field}: {err}"))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let program = args.first().map_or("egorec-mp4", String::as_str);

    let result = match args.as_slice() {
        [_, input, output] => convert::build_mp4_file(&PathBuf::from(input), &PathBuf::from(output))
            .map_err(|err| (err.exit_code(), err.to_string())),
        [_, command, input, output] if command == "mp4" => {
            convert::build_mp4_file(&PathBuf::from(input), &PathBuf::from(output))
                .map_err(|err| (err.exit_code(), err.to_string()))
        }
        [_, command, input, output, timescale, sample_delta] if command == "init" => {
            let timescale = parse_u32(timescale, "timescale")?;
            let sample_delta = parse_u32(sample_delta, "sample_delta")?;
            convert::build_init_file(
                &PathBuf::from(input),
                &PathBuf::from(output),
                timescale,
                sample_delta,
            )
            .map_err(|err| (err.exit_code(), err.to_string()))
        }
        [_, command, input, output, timescale, sample_delta, sequence_number, base_decode_time]
            if command == "segment" =>
        {
            let timescale = parse_u32(timescale, "timescale")?;
            let sample_delta = parse_u32(sample_delta, "sample_delta")?;
            let sequence_number = parse_u32(sequence_number, "sequence_number")?;
            let base_decode_time = parse_u64(base_decode_time, "base_decode_time")?;
            convert::build_segment_file(
                &PathBuf::from(input),
                &PathBuf::from(output),
                timescale,
                sample_delta,
                sequence_number,
                base_decode_time,
            )
            .map_err(|err| (err.exit_code(), err.to_string()))
        }
        _ => {
            return Err(usage(program));
        }
    };

    match result {
        Ok(()) => Ok(()),
        Err((exit_code, message)) => {
            eprintln!("{message}");
            std::process::exit(exit_code);
        }
    }
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(64);
    }
}
