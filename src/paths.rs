//! Filesystem rules: which files are media, what a valid folder path is, and
//! how files are moved, including between disks.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::error::bad_request;

pub const TRASH_DIR: &str = ".hoard-trash";

const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "m2ts", "mts",
    "3gp", "ogv",
];
const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "avif", "bmp"];

/// "video", "image", or None for anything the library does not import.
pub fn media_kind(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if VIDEO.contains(&ext.as_str()) {
        Some("video")
    } else if IMAGE.contains(&ext.as_str()) {
        Some("image")
    } else {
        None
    }
}

/// Hidden entries are never imported: that keeps the trash, partial files and
/// other tools' caches out of the library.
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Normalises a folder path relative to a root: forward slashes, no empty or
/// dot segments, nothing that could climb out of the root.
pub fn clean_folder(raw: &str) -> Result<String> {
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." || is_hidden(part) {
            return Err(bad_request(format!("invalid folder name {part:?}")).into());
        }
        if part.chars().any(|c| c.is_control()) {
            return Err(bad_request("folder names cannot contain control characters").into());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// A file name safe to create: no separators, not hidden, not empty.
pub fn clean_filename(raw: &str) -> Result<String> {
    let name: String = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    let name = name.trim().trim_start_matches('.').to_string();
    if name.is_empty() {
        return Err(bad_request("empty file name").into());
    }
    Ok(name)
}

/// A GLOB pattern for everything below `folder`. GLOB rather than LIKE:
/// LIKE ignores case, and `clips` must not take `Clips/…` along with it.
pub fn below_glob(folder: &str) -> String {
    let escaped: String = folder
        .chars()
        .map(|c| if matches!(c, '*' | '?' | '[') { format!("[{c}]") } else { c.to_string() })
        .collect();
    format!("{escaped}/*")
}

/// The directory of a folder path under a root.
pub fn folder_dir(root: &Path, folder: &str) -> PathBuf {
    if folder.is_empty() { root.to_path_buf() } else { root.join(folder) }
}

/// Splits a path relative to the root into (folder, filename).
pub fn split_relative(root: &Path, file: &Path) -> Option<(String, String)> {
    let rel = file.strip_prefix(root).ok()?;
    let filename = rel.file_name()?.to_str()?.to_string();
    let mut parts = Vec::new();
    for c in rel.parent()?.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    Some((parts.join("/"), filename))
}

/// `name.ext`, or `name (1).ext`, `name (2).ext`… — whichever does not exist yet.
pub fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(filename);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let ext = path.extension().and_then(|s| s.to_str());
    (1..)
        .map(|i| match ext {
            Some(ext) => dir.join(format!("{stem} ({i}).{ext}")),
            None => dir.join(format!("{stem} ({i})")),
        })
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// How much of an upload may sit in memory before it is flushed to disk.
const FLUSH_EVERY: u64 = 32 << 20;

/// A file an upload streams into, flushed to disk as it grows.
///
/// Left alone, the kernel holds gigabytes of an upload in memory, then blocks
/// the writer for as long as a slow disk takes to catch up. Nothing reads the
/// connection meanwhile, and a proxy drops it as dead (HTTP/3 after 30s idle).
/// Flushing as it goes trades that one long pause for many short ones.
pub struct UploadFile {
    file: tokio::fs::File,
    unflushed: u64,
}

impl UploadFile {
    pub async fn create(path: &Path) -> io::Result<Self> {
        Ok(Self { file: tokio::fs::File::create(path).await?, unflushed: 0 })
    }

    pub async fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.file.write_all(chunk).await?;
        self.unflushed += chunk.len() as u64;
        if self.unflushed >= FLUSH_EVERY {
            self.file.sync_data().await?;
            self.unflushed = 0;
        }
        Ok(())
    }

    pub async fn finish(self) -> io::Result<()> {
        self.file.sync_all().await
    }
}

/// Files next to a media file that belong to it: subtitles, `.info.json`,
/// `.nfo` — anything sharing its stem that is not itself media.
pub fn companions(file: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (file.parent(), file.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.");
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p != file && p.is_file() && media_kind(p).is_none())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
        .collect()
}

/// Moves a file, falling back to copy-and-delete when the destination is on
/// another filesystem. `progress` is called with bytes copied so far.
///
/// The copy goes to a hidden temporary name first and is renamed into place
/// only when complete, so a crash mid-copy never leaves a truncated file that
/// the next scan would import.
pub fn move_file(from: &Path, to: &Path, mut progress: impl FnMut(u64)) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    match fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) if is_cross_device(&e) => {}
        Err(e) => return Err(e).with_context(|| format!("moving {}", from.display())),
    }

    let meta = fs::metadata(from)?;
    let tmp = to.with_file_name(format!(
        ".{}.hoard-part",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("file")
    ));
    let result = (|| -> Result<()> {
        let mut src = fs::File::open(from)?;
        let mut dst = fs::File::create(&tmp)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut copied = 0u64;
        loop {
            let n = io::Read::read(&mut src, &mut buf)?;
            if n == 0 {
                break;
            }
            io::Write::write_all(&mut dst, &buf[..n])?;
            copied += n as u64;
            progress(copied);
        }
        dst.sync_all()?;
        drop(dst);
        // Keep the original modification time: the scanner recognises a file
        // that moved behind its back by size and mtime.
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        filetime::set_file_mtime(&tmp, mtime)?;
        fs::rename(&tmp, to)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("copying {} to {}", from.display(), to.display()));
    }
    fs::remove_file(from).with_context(|| format!("removing {} after copy", from.display()))?;
    Ok(())
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on unix, ERROR_NOT_SAME_DEVICE on windows.
    e.kind() == io::ErrorKind::CrossesDevices
}

/// Moves a whole directory, everything in it included. On one filesystem
/// that is a single rename; across filesystems the tree is copied into a
/// hidden directory next to the target, renamed into place once complete,
/// and only then is the original removed — an interrupted copy leaves the
/// source untouched. `progress` gets the bytes copied of `total`.
pub fn move_dir(from: &Path, to: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    if to.exists() {
        anyhow::bail!("{} already exists", to.display());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    match fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) if is_cross_device(&e) => {}
        Err(e) => return Err(e).with_context(|| format!("moving {}", from.display())),
    }

    let total = dir_size(from);
    let tmp = to.with_file_name(format!(
        ".{}.hoard-part",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("folder")
    ));
    let _ = fs::remove_dir_all(&tmp);
    let mut copied = 0u64;
    let result = copy_tree(from, &tmp, &mut |n| {
        copied += n;
        progress(copied, total);
    })
    .and_then(|()| fs::rename(&tmp, to).map_err(Into::into));
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e).with_context(|| format!("copying {} to {}", from.display(), to.display()));
    }
    fs::remove_dir_all(from).with_context(|| format!("removing {} after copy", from.display()))
}

/// Copies a directory tree, keeping modification times: the scanner knows a
/// file by its size and mtime.
fn copy_tree(from: &Path, to: &Path, copied: &mut impl FnMut(u64)) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&src, &dst, copied)?;
        } else if kind.is_file() {
            let mut input = fs::File::open(&src)?;
            let mut output = fs::File::create(&dst)?;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = io::Read::read(&mut input, &mut buf)?;
                if n == 0 {
                    break;
                }
                io::Write::write_all(&mut output, &buf[..n])?;
                copied(n as u64);
            }
            output.sync_all()?;
            let meta = fs::metadata(&src)?;
            filetime::set_file_mtime(&dst, filetime::FileTime::from_last_modification_time(&meta))?;
        }
        // Symlinks and other special files are not part of a media library;
        // they would only be followed somewhere unexpected.
    }
    Ok(())
}

/// Moves a file into its root's trash, under a dated directory so the purge
/// can expire whole days at once. Renaming within the same filesystem makes
/// this instant even for large videos.
pub fn trash_file(root: &Path, file: &Path) -> Result<()> {
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dir = root.join(TRASH_DIR).join(day);
    fs::create_dir_all(&dir)?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let target = unique_path(&dir, name);
    move_file(file, &target, |_| {})
}

/// Removes trash days older than `days`. With zero, everything goes.
pub fn purge_trash(root: &Path, days: u64) -> Result<u64> {
    let dir = root.join(TRASH_DIR);
    let Ok(entries) = fs::read_dir(&dir) else { return Ok(0) };
    let cutoff = chrono::Local::now().date_naive() - chrono::Days::new(days);
    let mut freed = 0;
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let expired = chrono::NaiveDate::parse_from_str(&name, "%Y-%m-%d")
            .map(|d| days == 0 || d < cutoff)
            .unwrap_or(false);
        if expired {
            freed += dir_size(&entry.path());
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(freed)
}

pub fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_normalised() {
        assert_eq!(clean_folder("/a//b/ c /").unwrap(), "a/b/c");
        assert_eq!(clean_folder("").unwrap(), "");
        assert!(clean_folder("a/../b").is_err());
        assert!(clean_folder(".hoard-trash").is_err());
    }

    #[test]
    fn globs_match_only_below_the_folder() {
        assert_eq!(below_glob("a/b"), "a/b/*");
        assert_eq!(below_glob("x*[1]?"), "x[*][[]1][?]/*");
    }

    #[test]
    fn relative_paths_split() {
        let root = Path::new("/media");
        assert_eq!(
            split_relative(root, Path::new("/media/a/b/c.mp4")),
            Some(("a/b".into(), "c.mp4".into()))
        );
        assert_eq!(split_relative(root, Path::new("/media/c.mp4")), Some(("".into(), "c.mp4".into())));
    }

    /// The cross-filesystem path of `move_dir`, which a test cannot force.
    #[test]
    fn trees_copy_with_their_mtimes() {
        let dir = std::env::temp_dir().join(format!("hoard-copy-test-{}", std::process::id()));
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("top.mp4"), b"12345").unwrap();
        fs::write(src.join("a/b/deep.png"), b"xy").unwrap();
        let old = filetime::FileTime::from_unix_time(1_600_000_000, 0);
        filetime::set_file_mtime(src.join("a/b/deep.png"), old).unwrap();

        let mut copied = 0;
        copy_tree(&src, &dst, &mut |n| copied += n).unwrap();
        assert_eq!(copied, 7);
        assert_eq!(fs::read(dst.join("top.mp4")).unwrap(), b"12345");
        let meta = fs::metadata(dst.join("a/b/deep.png")).unwrap();
        assert_eq!(filetime::FileTime::from_last_modification_time(&meta), old);

        assert!(move_dir(&src, &dst, |_, _| {}).is_err(), "never merges into an existing folder");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn uploads_survive_flushing_midway() {
        let path = std::env::temp_dir().join(format!("hoard-upload-test-{}", std::process::id()));
        let chunk = vec![7u8; (FLUSH_EVERY / 3 + 1) as usize];
        let mut file = UploadFile::create(&path).await.unwrap();
        for _ in 0..4 {
            file.write(&chunk).await.unwrap();
        }
        assert!(file.unflushed < FLUSH_EVERY, "flushed once past the threshold");
        file.finish().await.unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 4 * chunk.len() as u64);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unique_names_count_up() {
        let dir = std::env::temp_dir().join(format!("hoard-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.mp4"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.mp4"), dir.join("a (1).mp4"));
        assert_eq!(unique_path(&dir, "b.mp4"), dir.join("b.mp4"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
