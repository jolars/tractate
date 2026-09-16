use std::collections::BTreeMap;
use std::fs;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::*;
use crate::document::{Origin, SemanticIds, SourceFile, SourceRange};
use crate::render::html::RenderedSlide;

fn slides() -> Vec<RenderedSlide> {
    let ids = SemanticIds::default();
    let file = SourceFile::anonymous("## One\n\n## Two\n");
    ["One", "Two"]
        .into_iter()
        .map(|body| {
            let id = ids.slide();
            RenderedSlide {
                id,
                origin: Origin::Source(
                    file.span(SourceRange {
                        start: 0,
                        end: file.text().len(),
                    })
                    .unwrap(),
                ),
                html: format!("<section data-slide-id=\"slide-{id}\"><h2>{body}</h2></section>\n"),
            }
        })
        .collect()
}

fn deck() -> HtmlDirectory {
    HtmlDirectory::new("A deck", &slides(), &[]).unwrap()
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn html_directory_emits_complete_deck_and_content_manifest() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    let slides = slides();
    let asset = HtmlAsset {
        path: "figures/plot.svg",
        bytes: b"<svg></svg>",
    };
    HtmlDirectory::new("A & </title> café", &slides, &[asset])
        .unwrap()
        .write(&output)
        .unwrap();

    let files = tree(&output);
    let html = std::str::from_utf8(&files["index.html"]).unwrap();
    assert!(html.starts_with("<!doctype html>\n"));
    assert!(html.contains("<meta charset=\"utf-8\">"));
    assert!(html.contains("name=\"viewport\""));
    assert!(html.contains("<title>A &amp; &lt;/title&gt; café</title>"));
    assert!(html.contains("<div class=\"reveal\">\n<div class=\"slides\">"));
    assert!(html.contains(&slides.iter().map(|s| s.html.as_str()).collect::<String>()));
    assert!(html.contains("plugins: [RevealMath.KaTeX]"));
    assert!(html.contains("version: \"0.16.22\""));
    assert!(!html.contains("latest"));
    for path in [
        "dist/reset.css",
        "dist/reveal.css",
        "dist/theme/serif.css",
        "dist/reveal.js",
        "plugin/math/math.js",
    ] {
        assert!(html.contains(&format!(
            "https://cdn.jsdelivr.net/npm/reveal.js@5.2.1/{path}"
        )));
    }
    assert_eq!(files["figures/plot.svg"], b"<svg></svg>");

    let manifest: Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    assert_eq!(manifest["generator"], "tractate");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["entrypoint"], "index.html");
    let entries = manifest["files"].as_array().unwrap();
    assert_eq!(entries.len(), files.len() - 1);
    for entry in entries {
        let path = entry["path"].as_str().unwrap();
        assert_ne!(path, "manifest.json");
        let bytes = &files[path];
        assert_eq!(entry["bytes"], bytes.len());
        assert_eq!(entry["sha256"], format!("{:x}", Sha256::digest(bytes)));
    }
    let entries = manifest["slides"].as_array().unwrap();
    assert_eq!(entries.len(), slides.len());
    for (entry, slide) in entries.iter().zip(slides) {
        assert_eq!(entry["id"], format!("slide-{}", slide.id));
        assert_eq!(
            entry["sha256"],
            format!("{:x}", Sha256::digest(slide.html.as_bytes()))
        );
    }
    assert_eq!(manifest["external_assets"][0]["version"], "5.2.1");
    assert_eq!(manifest["external_assets"][1]["version"], "0.16.22");
    assert_eq!(
        manifest["external_assets"][1]["base_url"],
        "https://cdn.jsdelivr.net/npm/katex@0.16.22/"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn html_directory_is_deterministic_and_tracks_content_changes() {
    let root = tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    let assets = [
        HtmlAsset {
            path: "z.txt",
            bytes: b"z",
        },
        HtmlAsset {
            path: "a.txt",
            bytes: b"a",
        },
    ];
    HtmlDirectory::new("Title", &slides(), &assets)
        .unwrap()
        .write(&a)
        .unwrap();
    HtmlDirectory::new("Title", &slides(), &[assets[1], assets[0]])
        .unwrap()
        .write(&b)
        .unwrap();
    assert_eq!(tree(&a), tree(&b));
    let original: Value =
        serde_json::from_slice(&fs::read(a.join("manifest.json")).unwrap()).unwrap();
    let mut revised = slides();
    revised[1].html = revised[1].html.replace("Two", "Revised");
    HtmlDirectory::new("Title", &revised, &assets)
        .unwrap()
        .write(&b)
        .unwrap();
    let changed: Value =
        serde_json::from_slice(&fs::read(b.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(original["slides"][0], changed["slides"][0]);
    assert_eq!(original["slides"][1]["id"], changed["slides"][1]["id"]);
    assert_ne!(
        original["slides"][1]["sha256"],
        changed["slides"][1]["sha256"]
    );
}

#[test]
fn html_directory_replaces_an_existing_deck_and_removes_stale_assets() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    HtmlDirectory::new(
        "Old",
        &slides(),
        &[HtmlAsset {
            path: "old.txt",
            bytes: b"old",
        }],
    )
    .unwrap()
    .write(&output)
    .unwrap();
    HtmlDirectory::new("New", &[], &[])
        .unwrap()
        .write(&output)
        .unwrap();
    assert!(!output.join("old.txt").exists());
    let html = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(html.contains("<title>New</title>"));
    assert!(!html.contains("<section"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn html_directory_rejects_unsafe_or_colliding_asset_paths() {
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../b",
        "a/./b",
        "./a",
        "a//b",
        "a/",
        "a\\b",
        "C:/escape",
        "nul\0byte",
        "index.html",
        "manifest.json",
        "index.html/child",
    ] {
        let result = HtmlDirectory::new(
            "Title",
            &slides(),
            &[HtmlAsset {
                path,
                bytes: b"asset",
            }],
        );
        assert!(result.is_err(), "accepted {path:?}");
    }
    for paths in [["a", "a"], ["a", "a/b"], ["a/b", "a"]] {
        assert!(
            HtmlDirectory::new(
                "Title",
                &[],
                &paths.map(|path| HtmlAsset { path, bytes: b"" })
            )
            .is_err()
        );
    }
    let mut duplicate = slides();
    duplicate.push(duplicate[0].clone());
    assert!(HtmlDirectory::new("Title", &duplicate, &[]).is_err());
}

#[test]
fn html_directory_preserves_unrelated_paths_and_cleans_staging_on_publication_error() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("notes.txt"), "keep me").unwrap();
    let before = tree(root.path());
    assert!(deck().write(&output).is_err());
    assert_eq!(tree(root.path()), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);

    let file = root.path().join("file");
    fs::write(&file, "keep me").unwrap();
    assert!(deck().write(&file).is_err());
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
    assert!(deck().write(&file.join("deck")).is_err());
}

#[test]
fn html_directory_partial_staging_failure_leaves_published_output_unchanged() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    deck().write(&output).unwrap();
    let before = tree(&output);
    {
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        fs::create_dir(staging.path().join("index.html")).unwrap();
        let revised = HtmlDirectory::new(
            "New",
            &[],
            &[HtmlAsset {
                path: "a.txt",
                bytes: b"partial",
            }],
        )
        .unwrap();
        assert!(revised.write_files(staging.path()).is_err());
        assert!(staging.path().join("a.txt").is_file());
        assert!(!staging.path().join("manifest.json").exists());
        assert_eq!(tree(&output), before);
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn html_directory_staging_never_overwrites_existing_files() {
    for path in ["index.html", "manifest.json"] {
        let staging = tempdir().unwrap();
        fs::write(staging.path().join(path), "existing bytes").unwrap();
        assert!(deck().write_files(staging.path()).is_err());
        assert_eq!(
            fs::read_to_string(staging.path().join(path)).unwrap(),
            "existing bytes"
        );
    }
}

#[test]
fn html_directory_failed_atomic_commit_preserves_the_previous_deck() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    deck().write(&output).unwrap();
    let before = tree(root.path());
    assert!(publish(&root.path().join("missing-staging"), &output).is_err());
    assert_eq!(tree(root.path()), before);
    assert!(
        publish(
            &root.path().join("missing-staging"),
            &root.path().join("new")
        )
        .is_err()
    );
    assert!(!root.path().join("new").exists());
}

#[test]
fn html_directory_rejects_corrupt_and_foreign_manifests() {
    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    fs::create_dir(&output).unwrap();
    for manifest in [
        "{",
        "{}",
        r#"{"generator":"other","schema_version":1,"entrypoint":"index.html"}"#,
        r#"{"generator":"tractate","schema_version":2,"entrypoint":"index.html"}"#,
    ] {
        fs::write(output.join("manifest.json"), manifest).unwrap();
        let before = tree(root.path());
        assert!(deck().write(&output).is_err());
        assert_eq!(tree(root.path()), before);
    }
}

#[cfg(unix)]
#[test]
fn html_directory_commit_exchanges_complete_directories() {
    use std::os::unix::fs::MetadataExt;

    let root = tempdir().unwrap();
    let output = root.path().join("deck");
    deck().write(&output).unwrap();
    let before = tree(&output);
    let old_inode = fs::metadata(&output).unwrap().ino();
    let staging = tempfile::tempdir_in(root.path()).unwrap();
    let next = HtmlDirectory::new("New", &[], &[]).unwrap();
    next.write_files(staging.path()).unwrap();
    let after = tree(staging.path());
    let next_inode = fs::metadata(staging.path()).unwrap().ino();
    assert_eq!(tree(&output), before);
    publish(staging.path(), &output).unwrap();
    assert_eq!(fs::metadata(&output).unwrap().ino(), next_inode);
    assert_eq!(fs::metadata(staging.path()).unwrap().ino(), old_inode);
    assert_eq!(tree(&output), after);
    assert_eq!(tree(staging.path()), before);
}

#[cfg(unix)]
#[test]
fn html_directory_rejects_output_symlinks_without_touching_their_targets() {
    let root = tempdir().unwrap();
    let target = root.path().join("target");
    deck().write(&target).unwrap();
    let before = tree(&target);
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(deck().write(&link).is_err());
    assert_eq!(tree(&target), before);
    assert!(link.is_symlink());
    let dangling = root.path().join("dangling");
    std::os::unix::fs::symlink(root.path().join("missing"), &dangling).unwrap();
    assert!(deck().write(&dangling).is_err());
    assert!(dangling.is_symlink());
}
