//! avi2mp4 — convert video and audio files between formats (AVI -> MP4 by default).
//!
//! Run with no arguments to open the GUI; pass files to use the command line.

mod convert;
mod gui;

use convert::{Format, Options, FORMATS, PRESETS};
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
avi2mp4 - convert video and audio files between formats

USAGE:
    avi2mp4 [OPTIONS] <FILE|DIR>...

    A directory argument converts every media file inside it
    (except files already in the target format).

OUTPUT FORMAT:
    -f, --format <FMT>        Target format (default: mp4)
                              video: mp4 mov mkv webm avi gif
                              audio: mp3 m4a wav flac ogg opus

INPUT SELECTION:
        --from <EXT,...>      Only pick these extensions from directories,
                              e.g. --from avi,wmv
    -r, --recursive           Also search subdirectories

OUTPUT:
    -o, --out-dir <DIR>       Write files to DIR (default: next to the input)
    -y, --overwrite           Overwrite existing output files

VIDEO:
    -c, --crf <N>             Quality, lower = better
                              (h264: 0-51, default 20; vp9: 0-63, default 31)
    -p, --preset <NAME>       Speed: ultrafast..veryslow (default: medium)
    -e, --encode              Always re-encode video (never stream-copy)
    -w, --width <PX>          Resize to this width, keeping aspect ratio
        --fps <N>             Change the frame rate

TRIMMING:
    -s, --start <TIME>        Start at TIME (seconds, MM:SS or HH:MM:SS)
    -d, --duration <TIME>     Keep only this much (same formats)

AUDIO:
    -a, --no-audio            Drop the audio track
    -b, --audio-bitrate <R>   Bitrate for lossy audio encoding (default: 192k)

    -h, --help                Show this help

    Run with no arguments to open the window (GUI) instead.

ENVIRONMENT:
    FFMPEG_DIR                Folder containing ffmpeg/ffprobe if not on PATH
";

fn main() -> ExitCode {
    if env::args_os().len() == 1 {
        return gui::run();
    }

    let opts = match parse_args() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = convert::check_tools() {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    let files = match convert::collect_files(&opts) {
        Ok(f) if f.is_empty() => {
            eprintln!("error: no matching media files found");
            return ExitCode::FAILURE;
        }
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(dir) = &opts.out_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("error: cannot create {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    let total = files.len();
    let (mut failed, mut skipped) = (0, 0);
    for (i, file) in files.iter().enumerate() {
        println!("[{}/{}] {} -> {}", i + 1, total, file.display(), opts.format.ext());
        let mut showed_progress = false;
        let mut report = |r: convert::Report| match r {
            convert::Report::Streams(s) => println!("    {s}"),
            convert::Report::Progress(pct) => {
                print!("\r    progress: {pct:5.1}%");
                let _ = std::io::stdout().flush();
                showed_progress = true;
            }
        };
        let result = convert::convert(file, &opts, &mut report);
        if showed_progress {
            println!();
        }
        match result {
            Ok(Some(out)) => println!("    -> {}", out.display()),
            Ok(None) => {
                println!("    skipped (output exists, use -y to overwrite)");
                skipped += 1;
            }
            Err(e) => {
                eprintln!("    FAILED: {e}");
                failed += 1;
            }
        }
    }

    println!(
        "\nDone: {} converted, {} skipped, {} failed.",
        total - failed - skipped,
        skipped,
        failed
    );
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options::default();

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} requires a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-f" | "--format" => {
                let v = value(&arg)?;
                opts.format = Format::parse(&v).ok_or(format!(
                    "unknown format '{v}' (choose from: {})",
                    FORMATS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ))?;
            }
            "--from" => {
                opts.from = Some(
                    value(&arg)?
                        .split(',')
                        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect(),
                )
            }
            "-r" | "--recursive" => opts.recursive = true,
            "-o" | "--out-dir" => opts.out_dir = Some(PathBuf::from(value(&arg)?)),
            "-y" | "--overwrite" => opts.overwrite = true,
            "-c" | "--crf" => {
                opts.crf = Some(
                    value(&arg)?
                        .parse()
                        .ok()
                        .filter(|n| *n <= 63)
                        .ok_or("--crf must be a number 0-63")?,
                )
            }
            "-p" | "--preset" => {
                let v = value(&arg)?.to_ascii_lowercase();
                if !PRESETS.contains(&v.as_str()) {
                    return Err(format!("unknown preset '{v}' (choose from: {})", PRESETS.join(", ")));
                }
                opts.preset = v;
            }
            "-e" | "--encode" => opts.force_encode = true,
            "-w" | "--width" => {
                opts.width = Some(
                    value(&arg)?
                        .parse()
                        .ok()
                        .filter(|w| *w >= 2)
                        .ok_or("--width must be a whole number of pixels")?,
                )
            }
            "--fps" => {
                opts.fps = Some(
                    value(&arg)?
                        .parse()
                        .ok()
                        .filter(|f: &f64| *f > 0.0)
                        .ok_or("--fps must be a positive number")?,
                )
            }
            "-s" | "--start" => opts.start = Some(time_arg(value(&arg)?, "--start")?),
            "-d" | "--duration" => opts.duration = Some(time_arg(value(&arg)?, "--duration")?),
            "-a" | "--no-audio" => opts.no_audio = true,
            "-b" | "--audio-bitrate" => opts.audio_bitrate = value(&arg)?,
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => opts.inputs.push(PathBuf::from(arg)),
        }
    }

    if opts.inputs.is_empty() {
        return Err("no input files given".into());
    }
    if opts.no_audio && opts.format.is_audio() {
        return Err(format!("--no-audio makes no sense with the audio format {}", opts.format.ext()));
    }
    if let Some(crf) = opts.crf {
        if opts.format != Format::Webm && crf > 51 {
            return Err("--crf must be 0-51 for h264 output (0-63 is only for webm)".into());
        }
    }
    Ok(opts)
}

fn time_arg(v: String, name: &str) -> Result<String, String> {
    match convert::parse_time(&v) {
        Some(_) => Ok(v),
        None => Err(format!("{name}: '{v}' is not a time (use seconds, MM:SS or HH:MM:SS)")),
    }
}

