# rplayer

TUI-based highlights logger for mpv using JSON IPC. Review multiple videos
quickly, mark IN/OUT segments, and render a single highlight reel with ffmpeg.

## Requirements

- mpv on PATH
- ffmpeg + ffprobe on PATH

## Usage

From a folder containing `.mp4` files:

```text
cargo run
```

You can enable mpv debug logging to `mpv.log`:

```text
cargo run -- --debug-mpv
```

## Keybindings

General:

- `Space` pause / play (also used for volume chords)
- `h` / `l` seek -5s / +5s
- `H` / `L` seek -30s / +30s
- `j` / `k` speed -0.25 / +0.25
- `i` mark IN
- `o` mark OUT
- `u` undo last segment (current file)
- `n` next file (on last file, prompts to render)
- `p` previous file
- `q` export markers and quit
- `?` toggle help
- `Esc` clear pending IN

Volume chords (press Space then the key):

- `Space` + `v` volume down
- `Space` + `V` volume up
- `Space` + `m` mute toggle

Zoom mode:

- `z` enter zoom mode (pauses playback)
- `+` / `-` zoom in / out
- `h` / `j` / `k` / `l` pan left / down / up / right
- `0` reset zoom
- `q` exit zoom mode (restores prior pause state)

Markers editor:

- `Ctrl+g` open the `markers.json` editor
- `:q` + Enter to save and close
- If JSON is invalid: `f` to fix, `d` to discard and exit

## Output

On quit, the app writes:

- `markers.json` (all segments)
- `<basename>.cuts.csv` for each video with segments

When rendering from the last file prompt, it writes:

- `output/output_YYYYMMDD_HHMMSS.mp4`

## Notes

- All errors are logged to `app.log`.
- mpv output is logged to `mpv.log`.
