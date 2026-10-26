# hoard

A self-hosted media library for videos and clips, backed by plain folders.

Point it at one or more directories and it imports every video and image it
finds, with thumbnails, search, tags and a full-viewport player. Your files
stay where they are, in the folder structure you already have — hoard's own
data (a SQLite database, thumbnails, storyboards) lives in a separate data
directory.

- **Folders, not albums.** The library's folders are the directories on disk.
  Create, delete and move between them in the UI and the files move on disk
  too — including between storage folders on different drives.
- **Several storage folders.** Add as many directories as you like; each is
  scanned, and files reorganised outside the app are recognised as moved
  rather than deleted, so their tags survive.
- **Downloads with yt-dlp.** Paste a wall of links; each becomes a card you can
  tag and file while it downloads. yt-dlp is installed into its own Python
  environment in the data directory and kept up to date.
- **Search** across titles, file names, tags, descriptions and folders, with
  typo-tolerant tag matching: `beach -blurry tag:sunset -tag:draft folder:Trips/2024 kind:video`.
  Results appear as you type; qualifiers and their values (tags, folders,
  storage folders) complete with Tab; recent searches are remembered; on a
  folder's page the search is scoped to it until you widen it. `/` or
  Ctrl+K focuses the box, Ctrl+Enter opens in a new tab.
- **Tags**, with completion, and bulk tag / move / delete over a hand-picked
  selection or everything matching a search.
- **A proper viewer**: zoom and pan (wheel, pinch, double-click), custom
  controls with no overlay on pause, keyboard shortcuts, screenshots of the
  view or a region, and storyboards — a frame preview on the scrubber and a
  grid of the whole video — generated on request.
- **Storyboard queue.** Queue storyboards for a selection from the bulk bar,
  or for a whole folder (subfolders included) from its sidebar menu. The queue
  lives in the database, survives restarts, skips videos that already have
  one, and can be cancelled from the activity menu; a video opened in the
  viewer jumps the queue.
- **fiffly integration.** Open any video or image in the
  [fiffly](https://fiffly.com) browser editor from the viewer or a tile's ⋯
  menu, browse the library from inside fiffly, and save its exports back into
  a folder.
- **Optional password.** No accounts; set a password and every request needs
  a session.

## Requirements

- `ffmpeg` and `ffprobe` on `PATH` (probing, thumbnails, storyboards)
- Python 3 with `venv`, only for downloading from links
- To build: a current stable Rust and Node.js 20+

## Running

```sh
cd web && npm ci && npm run build && cd ..
cargo build --release
./target/release/hoard
```

Open <http://127.0.0.1:8150>, go to **Settings**, add a storage folder, and
install yt-dlp if you want to download from links.

The frontend is embedded into the binary at build time, so the release build
is a single file. Build the web app before `cargo build --release`.

### Docker

```sh
docker build -t hoard .
docker run -d -p 8150:8150 \
  -v hoard-data:/data \
  -v /path/to/videos:/media/videos \
  -e HOARD_ROOTS=/media/videos \
  -e HOARD_PASSWORD=change-me \
  hoard
```

## Configuration

All configuration is environment variables.

| Variable | Default | |
|---|---|---|
| `HOARD_LISTEN` | `127.0.0.1:8150` | Address to listen on |
| `HOARD_DATA_DIR` | `./data` | Database, thumbnails, storyboards, metadata, yt-dlp environment |
| `HOARD_PASSWORD` | — | Require this password |
| `HOARD_PASSWORD_HASH` | — | The same, as an argon2 hash from `hoard hash-password` |
| `HOARD_SECURE_COOKIES` | `false` | Mark the session cookie `Secure` (set behind HTTPS) |
| `HOARD_ROOTS` | — | Storage folders to register at startup, `:`-separated |
| `HOARD_SCAN_INTERVAL_MINUTES` | `60` | Rescan every storage folder this often; `0` disables |
| `HOARD_TRASH_DAYS` | `30` | Days deleted files stay in the trash; `0` deletes at once |
| `HOARD_WORKERS` | `2` | Thumbnails generated at once |
| `HOARD_DOWNLOADS` | `2` | Downloads running at once |
| `HOARD_FFMPEG` / `HOARD_FFPROBE` | `ffmpeg` / `ffprobe` | Paths to the binaries |
| `HOARD_PYTHON` | `python3` | Interpreter used to create the yt-dlp environment |
| `HOARD_YTDLP_ENV` | `$HOARD_DATA_DIR/ytdlp` | Where that environment lives |
| `HOARD_YTDLP_BIN` | — | Use this yt-dlp binary instead, unmanaged |
| `HOARD_YTDLP_AUTO_UPDATE` | `true` | Update the managed yt-dlp daily |
| `HOARD_LOG` | `hoard=info` | Log filter |

Without a password the server is open to anyone who can reach it; it warns at
startup when listening on a non-loopback address without one.

### yt-dlp

Installing from Settings creates a virtual environment and installs
`yt-dlp[default,deno]` into it — the extras bring the impersonation and
JavaScript runtime dependencies several sites require. A managed installation
is updated once a day. Extra arguments (format selection, `--cookies`, rate
limits) can be set in Settings; the default prefers H.264/AAC in MP4, which
every browser can play.

Downloads go to the folder chosen on the import page, the default download
folder from Settings, or the first storage folder. yt-dlp's metadata is kept
as `<data>/meta/<id>.info.json`.

### fiffly

fiffly runs entirely in the browser and talks to hoard over its media-publish
protocol. **Open in fiffly** links (viewer toolbar, ⋯ menu on tiles) carry a
link signed for about six hours and work without any setup. To browse the
library from fiffly and save exports into it, use **Settings → fiffly → Add
hoard to fiffly**: fiffly asks you to confirm, then to connect, which brings
you to a consent page in hoard. Connections are listed, and can be revoked, in
Settings.

Because fiffly runs in your browser, it reaches hoard at the address the
browser uses. fiffly accepts `https://` sites, or `http://` only on this
computer (`localhost`, `127.0.0.1`) — a LAN address like
`http://192.168.1.10:8150` won't work; put hoard behind https (a reverse
proxy) and enter that address in Settings. Chrome asks once whether fiffly
may access devices on your local network; allow it. A self-hosted fiffly can
be set as the fiffly address; only that address may request access.

What fiffly can reach: the manifest at `/.well-known/media-publish.json`, and
`/api/oauth/*` and `/api/publish/*`, which answer CORS for any origin and are
authorised by the token or the URL's signature, not the session cookie.

## How files are handled

- **Supported types:** mp4, m4v, mov, webm, mkv, avi, wmv, flv, mpg, mpeg, ts,
  m2ts, mts, 3gp, ogv; jpg, png, gif, webp, avif, bmp. Hidden files and
  directories are ignored. Files the browser cannot decode (often mkv/avi with
  older codecs) are listed and can be downloaded, but not played inline.
- **Delete** moves files into a `.hoard-trash` directory inside their storage
  folder, under a dated subdirectory; it is emptied after `HOARD_TRASH_DAYS`
  or from Settings.
- **Move** renames when source and destination share a filesystem and copies
  otherwise (to a hidden temporary name first, so an interrupted copy never
  looks like a real file). Name clashes get a numbered name rather than
  overwriting. Sidecar files sharing the media's name — subtitles,
  `.info.json`, `.nfo` — move with it.
- **Removing a storage folder** only forgets it; nothing on disk is touched.
- A storage folder that is missing or suddenly empty (an unmounted drive) is
  not treated as "everything was deleted": the scan stops and nothing is
  removed.

## Viewer shortcuts

| Key | |
|---|---|
| Space / k | Play / pause |
| ← / → | Seek 5 s |
| m | Mute |
| f | Fullscreen |
| b | Storyboard (builds it the first time) |
| s | Screenshot the visible view |
| r | Screenshot a region |
| Esc | Close |

Wheel or pinch to zoom about the pointer, drag to pan, double-click to toggle
2×. Screenshots are saved next to the original, with its tags.

## Development

```sh
cargo run                    # API on :8150
cd web && npm run dev        # UI on :5190, proxying to the API
cargo test
cd web && npm run typecheck
```

The layout: `src/api/` is the HTTP surface, `src/scan.rs` reconciles the
database with the disk, `src/ops.rs` moves and deletes files, `src/process.rs`
runs ffprobe/ffmpeg, `src/imports/` runs downloads, `src/search.rs` compiles
the query language to SQL over an FTS5 index, `web/` is the React frontend.

## License

MIT — see [LICENSE](LICENSE).
