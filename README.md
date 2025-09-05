# steamclipconverter

Convert Steam’s raw **DASH clips** (folders like `fg_294100_20250828_124021` containing a `session.mpd`) into **MP4** files using `ffmpeg`. Result files will be like

```
RimWorld-20250828-124021.mp4
```

---

## Features

- **Recursive scan** for clip folders like `fg_<appid>_<YYYYMMDD>_<HHMMSS>`  
- **MPD-based remux**: runs `ffmpeg -i session.mpd -map 0:v:0 -map 0:a:0? -c copy -movflags +faststart`  
- **Smart naming**: `GameName-YYYYMMDD-HHMMSS.mp4` (game name from `appmanifest_<appid>.acf`, fallback to AppID)  
- **Correct timestamps (UTC)**: output file’s modified time is set to the clip’s **record start in UTC** (Steam encodes UTC in the folder name)  
- **Filtering**: `--gameId 294100` (repeatable) to convert selected games only  
- **Cleanup**: `--delete-after` removes the `fg_*` directory and the corresponding the `clip_*` directory. Note: please restart Steam after delete clip, otherwise Steam will try to load these deleted clips.  
- **Cross‑platform Steam roots**: macOS, Linux, Windows (sane defaults; you can override with `--input`)

---

## Requirements

- **ffmpeg** in your `PATH`.

Quick installs:
```bash
# macOS
brew install ffmpeg

# Ubuntu/Debian
sudo apt-get update && sudo apt-get install -y ffmpeg

# Windows (PowerShell)
winget install Gyan.FFmpeg
```

---

## Install

### From crates.io (recommended)
```bash
cargo install steamclipconverter --locked
```

### From source
```bash
git clone https://github.com/qiaoruntao/steamclipconverter
cd steamclipconverter;
cargo build --release
./target/release/steamclipconverter --help
```

### Prebuilt binaries(not recommended)
These files are not signed and cannot run on openbox.

---

## Usage

### TL;DR
```bash
# Simplest: use default options and converted mp4 occurred in current directory
steamclipconverter

# Or pass flags explicitly
steamclipconverter --input "/path/to/Steam/userdata" --output "/path/to/out"
```

If you provide **no arguments**, the tool will **warn** and default to scanning your OS-specific Steam `userdata`:

- macOS: `~/Library/Application Support/Steam/userdata`  
- Linux: `~/.local/share/Steam/userdata`  
- Windows: `C:\Program Files (x86)\Steam\userdata`

Override anytime with `--input`.

---

### CLI reference

| Flag | Type | Default | Description |
|---|---|---|---|
| *(positional)* | path | — | If you pass exactly one non-flag argument, it’s treated as `--input`. |
| `--input` | path | *(OS default userdata if omitted, with warning)* | Root directory to scan **recursively** for `fg_*` clip folders. |
| `--output` | path | current working directory | Where to write `.mp4` files. |
| `--gameId` | u32 (repeatable) | *(all)* | Convert clips only for these **AppIDs**. Example: `--gameId 294100 --gameId 570`. |
| `--delete-after` | flag | off | After a **successful** convert, delete the `fg_*` folder; if it was the only folder under `video/`, also delete the `clip_*` grandparent. |

---

## How it works (straight talk)

1. **Find clips** – Recursively locate directories named `fg_<appid>_<YYYYMMDD>_<HHMMSS>`.
2. **Check MPD** – Ensure `session.mpd` exists inside each `fg_*` directory.
3. **Resolve game name** – Read `steamapps/appmanifest_<appid>.acf` from discovered Steam libraries (`libraryfolders.vdf` on all OSes). If missing, use the AppID.
4. **Mux** – Call `ffmpeg` on the **local** `session.mpd` and **stream copy** the first video + optional audio to MP4. No re-encode.
5. **Timestamp** – Set the output file’s mtime to the **record start (UTC)** parsed from the folder name (Steam stores UTC in `fg_<...>_YYYYMMDD_HHMMSS`).
6. **(Optional) Cleanup** – If `--delete-after`, remove the converted `fg_*` folder; if it was the **only** subdir in its parent `video/`, remove the `clip_*` grandparent too.

**About this common FFmpeg message**
```
[dash] Error when loading first fragment of playlist
```
Steam’s MPDs sometimes list extra **Representations** whose first fragment isn’t present (mismatched numbering, missing audio, etc.). FFmpeg drops the bad one and continues with a valid stream. The output is fine; you can safely ignore this warning.

---

## Examples

Convert everything under your Steam `userdata`, write to Desktop:
```bash
steamclipconverter --input "$HOME/Library/Application Support/Steam/userdata" \
                   --output "$HOME/Desktop/SteamClips"
```

Only convert RimWorld (294100) and Dota 2 (570):
```bash
steamclipconverter --input "$HOME/.local/share/Steam/userdata" \
                   --gameId 294100 --gameId 570
```

Convert then delete the source clip folders:
```bash
steamclipconverter --input "/path/to/Steam/userdata" --delete-after
```

---

## Expected folder layout

```
.../clip_294100_20250828_124021/
  └─ video/
     └─ fg_294100_20250828_124021/
        ├─ session.mpd
        ├─ init-stream0.m4s
        ├─ chunk-stream0-00001.m4s
        ├─ ...
        ├─ init-stream1.m4s         # (optional)
        └─ chunk-stream1-00001.m4s  # (optional)
```

> Only folders starting with **`fg_`** are processed. If audio is missing, the MP4 will be video‑only.

---

## References / Prior art

- Python original that inspired this approach (stitch + remux):  
  https://github.com/Nastas95/SteamClip/blob/main/steamclip.py

- Handy explanation of Steam’s clip structure & extracting:  
  https://gist.github.com/safijari/afa41cb017eb2d0cadb20bf9fcfecc93

- Community deep-dive on Steam Deck recordings:  
  https://www.reddit.com/r/SteamDeckTricks/comments/1dpj1zv/game_recording_how_it_works_and_how_you_can_get/

---

## License

MIT or Apache-2.0 (your call). Not affiliated with Valve/Steam.