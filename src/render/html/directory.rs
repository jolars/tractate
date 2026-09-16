//! Complete HTML output and its filesystem publication boundary.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{RenderedSlide, content::escape};

const REVEAL_VERSION: &str = "5.2.1";
const KATEX_VERSION: &str = "0.16.22";
const MANIFEST: &str = "manifest.json";

/// Caller-resolved local content, such as a source image or a result artifact.
/// Paths use forward slashes and are relative to the emitted directory.
#[derive(Clone, Copy)]
pub(crate) struct HtmlAsset<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// A complete, deterministic output snapshot, prepared without filesystem I/O.
pub(crate) struct HtmlDirectory {
    files: BTreeMap<String, Vec<u8>>,
    manifest: Vec<u8>,
}

impl HtmlDirectory {
    /// Assemble already rendered slides and caller-supplied local assets.
    ///
    /// The caller resolves the title, local resource paths, and cell results.
    /// Reveal and KaTeX use exact-version CDN URLs, including KaTeX's relative
    /// font URLs. Viewing requires network access; assembly and publication do
    /// not fetch assets, resolve computation, or execute code.
    pub fn new(
        title: &str,
        slides: &[RenderedSlide],
        assets: &[HtmlAsset<'_>],
    ) -> io::Result<Self> {
        let mut ids = HashSet::new();
        if slides.iter().any(|slide| !ids.insert(slide.id)) {
            return Err(invalid("Duplicate slide ID in HTML output"));
        }

        let external_assets = [
            ExternalAssets::new("reveal.js", REVEAL_VERSION),
            ExternalAssets::new("katex", KATEX_VERSION),
        ];
        let mut files = BTreeMap::from([(
            "index.html".to_owned(),
            index_html(title, slides, &external_assets[0].base_url).into_bytes(),
        )]);
        for asset in assets {
            validate_asset_path(asset.path)?;
            if files
                .insert(asset.path.to_owned(), asset.bytes.to_vec())
                .is_some()
            {
                return Err(invalid(format!("Duplicate output path: {}", asset.path)));
            }
        }
        // File/parent collisions would otherwise fail halfway through staging.
        for path in files.keys() {
            for (offset, _) in path.match_indices('/') {
                if files.contains_key(&path[..offset]) {
                    return Err(invalid(format!("Output file is also a directory: {path}")));
                }
            }
        }
        let manifest = ContentManifest {
            generator: "tractate",
            schema_version: 1,
            entrypoint: "index.html",
            files: files
                .iter()
                .map(|(path, bytes)| ManifestFile {
                    path,
                    bytes: bytes.len(),
                    sha256: digest(bytes),
                })
                .collect(),
            slides: slides
                .iter()
                .map(|slide| ManifestSlide {
                    id: format!("slide-{}", slide.id),
                    sha256: digest(slide.html.as_bytes()),
                })
                .collect(),
            external_assets,
        };
        let mut manifest = serde_json::to_vec_pretty(&manifest)?;
        manifest.push(b'\n');
        Ok(Self { files, manifest })
    }

    /// Publish the whole directory with one atomic rename or directory exchange.
    ///
    /// The parent directory must exist. Existing output must be a real directory
    /// bearing a Tractate manifest; unrelated paths and symlinks are rejected.
    /// Every error occurs before publication and leaves existing output intact.
    /// Staging and retired output are removed on drop on a best-effort basis.
    ///
    /// Linux, Android, and Apple targets use atomic rename flags. Other targets,
    /// or filesystems without the required operation, fail without a non-atomic
    /// fallback. This guarantees atomic visibility, not power-loss durability
    /// or a snapshot across several independent reader opens. Concurrent writers
    /// to the same destination must be serialized by the caller.
    pub fn write(&self, destination: &Path) -> io::Result<()> {
        let name = destination
            .file_name()
            .ok_or_else(|| invalid("Output needs a directory name"))?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize()?;
        let destination = parent.join(name);
        let staging = tempfile::Builder::new()
            .prefix(".tractate-html-")
            .tempdir_in(parent)?;
        self.write_files(staging.path())?;
        publish(staging.path(), &destination)
    }

    fn write_files(&self, directory: &Path) -> io::Result<()> {
        for (path, bytes) in &self.files {
            let path = directory.join(path);
            fs::create_dir_all(path.parent().expect("Output paths have a parent"))?;
            write_new(&path, bytes)?;
        }
        // The manifest describes only completed files and never hashes itself.
        write_new(&directory.join(MANIFEST), &self.manifest)
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Filesystem aliases, including case folding, can escape lexical checks.
    fs::File::create_new(path)?.write_all(bytes)
}

#[derive(Serialize)]
struct ContentManifest<'a> {
    generator: &'static str,
    schema_version: u32,
    entrypoint: &'static str,
    files: Vec<ManifestFile<'a>>,
    slides: Vec<ManifestSlide>,
    external_assets: [ExternalAssets; 2],
}

#[derive(Serialize)]
struct ManifestFile<'a> {
    path: &'a str,
    bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct ManifestSlide {
    id: String,
    sha256: String,
}

/// A pinned package root covers scripts, styles, and their relative resources.
#[derive(Serialize)]
struct ExternalAssets {
    package: &'static str,
    version: &'static str,
    base_url: String,
}

impl ExternalAssets {
    fn new(package: &'static str, version: &'static str) -> Self {
        Self {
            package,
            version,
            base_url: format!("https://cdn.jsdelivr.net/npm/{package}@{version}/"),
        }
    }
}

fn index_html(title: &str, slides: &[RenderedSlide], reveal_base: &str) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>",
    );
    escape(title, &mut html);
    html.push_str("</title>\n");
    // This theme uses system fonts, avoiding unversioned font-service imports.
    for path in ["dist/reset.css", "dist/reveal.css", "dist/theme/serif.css"] {
        html.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"{reveal_base}{path}\">\n"
        ));
    }
    html.push_str("</head>\n<body>\n<div class=\"reveal\">\n<div class=\"slides\">\n");
    for slide in slides {
        html.push_str(&slide.html);
    }
    html.push_str("</div>\n</div>\n");
    for path in ["dist/reveal.js", "plugin/math/math.js"] {
        html.push_str(&format!("<script src=\"{reveal_base}{path}\"></script>\n"));
    }
    // Only explicit math delimiters are typeset, so ordinary dollar text stays prose.
    html.push_str(&format!(
        r#"<script>
Reveal.initialize({{
  hash: true,
  katex: {{
    version: "{KATEX_VERSION}",
    delimiters: [
      {{ left: '\\(', right: '\\)', display: false }},
      {{ left: '\\[', right: '\\]', display: true }}
    ]
  }},
  plugins: [RevealMath.KaTeX]
}});
</script>
</body>
</html>
"#
    ));
    html
}

fn validate_asset_path(path: &str) -> io::Result<()> {
    if path.contains(['\\', ':', '\0'])
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        || path.split('/').next() == Some(MANIFEST)
    {
        return Err(invalid(format!("Invalid output asset path: {path:?}")));
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn is_existing_output(destination: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() {
        return Err(invalid("HTML output destination must be a real directory"));
    }
    let manifest_path = destination.join(MANIFEST);
    if !fs::symlink_metadata(&manifest_path)?.is_file() {
        return Err(invalid("Existing output manifest must be a regular file"));
    }
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest["generator"] != "tractate"
        || manifest["schema_version"] != 1
        || manifest["entrypoint"] != "index.html"
    {
        return Err(invalid(
            "Refusing to replace a directory without a Tractate HTML manifest",
        ));
    }
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn publish(staging: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    let flags = if is_existing_output(destination)? {
        RenameFlags::EXCHANGE
    } else {
        RenameFlags::NOREPLACE
    };
    // After an exchange, the TempDir owns the retired output at the staging path.
    // Nothing fallible follows the commit; cleanup cannot report a failed render.
    renameat_with(CWD, staging, CWD, destination, flags).map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn publish(_staging: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Atomic HTML directory publication is unsupported on this platform",
    ))
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "android", target_vendor = "apple")
))]
mod tests;
