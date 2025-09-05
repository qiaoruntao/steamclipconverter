use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use clap::{ArgAction, Parser};
use filetime::{set_file_times, FileTime};
use regex::Regex;
use sanitize_filename::sanitize;
use std::{
    collections::HashSet,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// OS-specific default Steam root candidates (NOT steamapps; that's added later).
macro_rules! steam_default_root_candidates {
    () => {{
        let mut v: Vec<PathBuf> = Vec::new();
        #[cfg(target_os = "macos")]
        {
            if let Ok(home) = std::env::var("HOME") {
                v.push(PathBuf::from(format!(
                    "{home}/Library/Application Support/Steam"
                )));
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(home) = std::env::var("HOME") {
                v.push(PathBuf::from(format!("{home}/.local/share/Steam")));
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(pf86) = std::env::var("PROGRAMFILES(X86)") {
                v.push(PathBuf::from(format!(r"{pf86}\Steam")));
            } else {
                v.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
            }
        }
        v
    }};
}

#[derive(Parser, Debug)]
#[command(
    name = "steamclipconverter",
    about = "Convert Steam 'fg_*' clip folders (with session.mpd) to MP4"
)]
struct Cli {
    /// Positional shorthand for --input. If present alone, treated as --input.
    input_positional: Option<PathBuf>,

    /// Directory to search recursively. If omitted, defaults to <SteamRoot>/userdata with a warning.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Output directory (defaults to current working directory)
    #[arg(long)]
    output: Option<PathBuf>,

    /// Restrict to specific appids; repeatable: --gameId 294100 --gameId 570
    #[arg(long = "gameId", action = ArgAction::Append)]
    game_ids: Vec<u32>,

    /// After successful conversion, delete the fg_... folder; if it was the only subdir
    /// in its parent 'video' dir, also delete its grandparent 'clip_<appid>_<date>_<time>' dir.
    #[arg(long, action = ArgAction::SetTrue)]
    delete_after: bool,
}

fn main() {
    // Allow "single positional only" to behave like --input.
    let argv: Vec<String> = env::args().collect();
    let mut argv_for_clap = argv.clone();
    if argv.len() == 2 && !argv[1].starts_with('-') {
        argv_for_clap = vec![argv[0].clone(), "--input".into(), argv[1].clone()];
    }
    let cli = Cli::parse_from(argv_for_clap);

    // Determine input directory.
    let input_dir = if let Some(p) = cli.input.or(cli.input_positional) {
        p
    } else {
        // No input provided: default to <SteamRoot>/userdata and WARN.
        let candidates = steam_default_root_candidates!();
        let chosen_root = candidates
            .iter()
            .find(|p| p.is_dir())
            .cloned()
            .or_else(|| candidates.get(0).cloned());
        match chosen_root {
            Some(root) => {
                let userdata = root.join("userdata");
                eprintln!(
                    "[warn] No --input provided. Defaulting to Steam userdata: {}\n       (OS defaults searched: {})\n       Pass --input \"<dir>\" to override.",
                    userdata.display(),
                    candidates
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                userdata
            }
            None => {
                eprintln!(
                    "ERROR: No --input provided and no recognizable default Steam root found for this OS.\n\
                     Try: --input \"/path/to/Steam/userdata\""
                );
                std::process::exit(2);
            }
        }
    };

    if !input_dir.is_dir() {
        eprintln!("ERROR: input is not a directory: {}", input_dir.display());
        std::process::exit(2);
    }

    let output_dir = cli
        .output
        .unwrap_or_else(|| env::current_dir().expect("cwd"));
    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!(
            "ERROR: cannot create output dir {}: {}",
            output_dir.display(),
            e
        );
        std::process::exit(2);
    }

    // Discover steamapps roots (for app-name lookup), across platforms.
    let steamapps_roots = discover_steamapps_roots();

    // Step 1: recursively find fg_* clip folders
    let mut clips = match find_fg_clip_dirs(&input_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR[find]: {}", e);
            std::process::exit(1);
        }
    };
    if clips.is_empty() {
        eprintln!("No fg_* clip folders found under {}", input_dir.display());
        std::process::exit(0);
    }

    // Optional filter by --gameId
    if !cli.game_ids.is_empty() {
        let set: HashSet<u32> = cli.game_ids.into_iter().collect();
        clips.retain(|c| set.contains(&c.appid));
    }

    if clips.is_empty() {
        println!("Nothing to convert after --gameId filtering.");
        std::process::exit(0);
    }

    // Deterministic order
    clips.sort_by(|a, b| a.dir.cmp(&b.dir));

    println!("Found {} clip folder(s).", clips.len());

    for clip in clips {
        println!(
            "== {} (appid={}, start={} {}) ==",
            clip.dir.display(),
            clip.appid,
            clip.date,
            clip.time
        );

        let mpd = clip.dir.join("session.mpd");
        if !mpd.is_file() {
            eprintln!("[skip] missing session.mpd");
            continue;
        }

        // Resolve game name (best-effort)
        let game_name = resolve_app_name(clip.appid, &steamapps_roots)
            .unwrap_or_else(|| clip.appid.to_string());

        // Filename: GameName-YYYYMMDD-HHMMSS.mp4  (sanitize for safety)
        let fname = format!("{}-{}-{}.mp4", sanitize(&game_name), clip.date, clip.time);
        let out_path = output_dir.join(&fname);

        println!("converting to {}", out_path.display());

        // Remux via ffmpeg using the local MPD.
        let status = Command::new("ffmpeg")
            .current_dir(&clip.dir) // MPD uses relative paths
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                "session.mpd",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                out_path.to_str().unwrap(),
            ])
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("[ok] wrote {}", out_path.display());

                // Set file times to the record start time (compact Chrono parse).
                if let Some(st) = to_systemtime(&clip.date, &clip.time) {
                    let ft = FileTime::from_system_time(st);
                    if let Err(e) = set_file_times(&out_path, ft, ft) {
                        eprintln!("[warn] failed to set file times: {}", e);
                        std::process::exit(2);
                    }
                } else {
                    eprintln!("[warn] could not parse start time for mtime");
                    std::process::exit(2);
                }

                // Delete-after semantics
                if cli.delete_after {
                    if let Err(e) = fs::remove_dir_all(&clip.dir) {
                        eprintln!("[warn] delete failed for {}: {}", clip.dir.display(), e);
                    } else {
                        println!("[del] removed {}", clip.dir.display());
                        maybe_remove_clip_grandparent(&clip);
                    }
                }
            }
            Ok(s) => {
                eprintln!("[fail] ffmpeg status: {}", s);
            }
            Err(e) => {
                eprintln!("[fail] launching ffmpeg: {}", e);
            }
        }
    }

    println!("\nDone.");
}

/// Represents one clip folder like fg_294100_20250828_124021
struct ClipDir {
    dir: PathBuf,
    appid: u32,
    date: String, // YYYYMMDD
    time: String, // HHMMSS
}

/// Recursively enumerate subfolders that match the fg_* pattern anywhere under `parent`.
fn find_fg_clip_dirs(parent: &Path) -> io::Result<Vec<ClipDir>> {
    let re = Regex::new(r"^fg_(\d+)_(\d{8})_(\d{6})$").unwrap();
    let mut out: Vec<ClipDir> = Vec::new();

    let mut stack: Vec<PathBuf> = vec![parent.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue, // skip unreadable dirs
        };

        for ent in entries.flatten() {
            let p = ent.path();
            if !p.is_dir() {
                continue;
            }

            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if let Some(caps) = re.captures(name) {
                    let appid: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
                    if appid != 0 {
                        let date = caps.get(2).unwrap().as_str().to_string();
                        let time = caps.get(3).unwrap().as_str().to_string();
                        out.push(ClipDir {
                            dir: p.clone(),
                            appid,
                            date,
                            time,
                        });
                    }
                    // clip folder is terminal; don't descend into it
                    continue;
                }
            }

            stack.push(p);
        }
    }

    Ok(out)
}

/// If fg dir was the ONLY directory in its parent 'video', also remove the 'clip_*' grandparent.
fn maybe_remove_clip_grandparent(clip: &ClipDir) {
    // parent should be .../video/
    let Some(video_dir) = clip.dir.parent() else {
        return;
    };
    if video_dir.file_name().and_then(|s| s.to_str()) != Some("video") {
        return;
    }

    // Are there any subdirectories left in video/ ?
    let mut any_left = false;
    if let Ok(rd) = fs::read_dir(video_dir) {
        for ent in rd.flatten() {
            if ent.path().is_dir() {
                any_left = true;
                break;
            }
        }
    }
    if any_left {
        return; // not the only one
    }

    // grandparent expected to be clip_<appid>_<date>_<time>
    let Some(clip_parent) = video_dir.parent() else {
        return;
    };
    if let Some(name) = clip_parent.file_name().and_then(|s| s.to_str()) {
        let re = Regex::new(r"^clip_\d+_\d{8}_\d{6}$").unwrap();
        if re.is_match(name) {
            match fs::remove_dir_all(clip_parent) {
                Ok(_) => println!("[del] removed {}", clip_parent.display()),
                Err(e) => eprintln!("[warn] failed to remove {}: {}", clip_parent.display(), e),
            }
        }
    }
}

/// Discover steamapps roots across OSes:
/// - default Steam roots from macro
/// - plus any additional libraries from libraryfolders.vdf (under <root>/config/ or <root>/steamapps/)
fn discover_steamapps_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    let steam_roots = steam_default_root_candidates!();
    for root in steam_roots {
        let sa = root.join("steamapps");
        if sa.is_dir() {
            roots.push(sa.clone());
        }

        let vdf1 = root.join("config").join("libraryfolders.vdf");
        let vdf2 = root.join("steamapps").join("libraryfolders.vdf");

        for vdf in [vdf1, vdf2] {
            if vdf.is_file() {
                if let Ok(txt) = fs::read_to_string(&vdf) {
                    for path in parse_libraryfolders_paths(&txt) {
                        let sp = Path::new(&path).join("steamapps");
                        if sp.is_dir() {
                            roots.push(sp);
                        }
                    }
                }
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

/// Extract library "path" values from libraryfolders.vdf
fn parse_libraryfolders_paths(vdf_text: &str) -> Vec<String> {
    // Accept lines like: "path" "/Volumes/External/SteamLibrary" or "path" "D:\\SteamLibrary"
    let path_re = Regex::new(r#""path"\s*"([^"]+)""#).unwrap();
    path_re
        .captures_iter(vdf_text)
        .map(|c| c[1].to_string())
        .collect()
}

/// Read appmanifest_<appid>.acf from any steamapps root and extract "name"
fn resolve_app_name(appid: u32, steamapps_roots: &[PathBuf]) -> Option<String> {
    let manifest = format!("appmanifest_{}.acf", appid);
    for root in steamapps_roots {
        let p = root.join(&manifest);
        if p.is_file() {
            if let Ok(txt) = fs::read_to_string(&p) {
                if let Some(name) = parse_acf_name(&txt) {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// Minimal ACF parser: `"name"   "Some Game"`
fn parse_acf_name(acf_text: &str) -> Option<String> {
    let re = Regex::new(r#""name"\s*"([^"]+)""#).unwrap();
    re.captures(acf_text).map(|c| c[1].to_string())
}

/// Convert to SystemTime assuming the clip's filename time is in **UTC**.
/// Inputs are "YYYYMMDD" and "HHMMSS" (already sliced from folder name).
fn to_systemtime(date8: &str, time6: &str) -> Option<std::time::SystemTime> {
    use std::time::{Duration, UNIX_EPOCH};

    let d = NaiveDate::parse_from_str(date8, "%Y%m%d").ok()?;
    let t = NaiveTime::parse_from_str(time6, "%H%M%S").ok()?;
    let ndt = NaiveDateTime::new(d, t);

    // Filenames are UTC; interpret naivedatetime as UTC then build SystemTime.
    let dt_utc = Utc.from_utc_datetime(&ndt);
    let secs = dt_utc.timestamp();
    let nanos = dt_utc.timestamp_subsec_nanos();

    Some(UNIX_EPOCH + Duration::from_secs(secs as u64) + Duration::from_nanos(nanos as u64))
}
