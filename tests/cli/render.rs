use std::fs;

use sha2::{Digest, Sha256};

use super::common::{TestProject, fixture};

fn static_project() -> TestProject {
    let project = TestProject::new(&fixture("static-deck.qmd"));
    fs::create_dir(project.root().join("figures")).unwrap();
    fs::write(
        project.root().join("figures/plot.svg"),
        fixture("figures/plot.svg"),
    )
    .unwrap();
    project
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_source_only_deck_with_manifest_and_local_resources() {
    let project = static_project();
    let output = project.run(&["render", "slides.qmd", "--to", "html"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.contains("slides_html/index.html"),
        "{output:?}"
    );
    assert!(output.stderr.is_empty(), "{output:?}");

    let destination = project.root().join("slides_html");
    let html = fs::read_to_string(destination.join("index.html")).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("<title>A static deck</title>"));
    assert_eq!(html.matches("<section data-slide-id=").count(), 4);
    assert!(html.contains("<strong>leading content</strong>"));
    assert!(html.contains("café and α"));
    assert!(html.contains("\\(x^2\\)"));
    assert!(html.contains("class=\"math display\">\\["));
    assert!(html.contains("<code class=\"language-r\">"));
    assert!(html.contains("stop(&quot;must not run&quot;)"));
    assert!(html.contains("reveal.js@5.2.1/"));
    assert!(html.contains("Reveal.initialize("));
    assert!(html.contains("RevealMath.KaTeX"));

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("manifest.json")).unwrap()).unwrap();
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(manifest["slides"].as_array().unwrap().len(), 4);
    assert_eq!(files.len(), 2);
    for file in files {
        let path = file["path"].as_str().unwrap();
        let bytes = fs::read(destination.join(path)).unwrap();
        assert_eq!(file["bytes"], bytes.len());
        assert_eq!(file["sha256"], format!("{:x}", Sha256::digest(&bytes)));
        if path != "index.html" {
            assert_eq!(bytes, fixture("figures/plot.svg").as_bytes());
            assert!(html.contains(&format!("src=\"{path}\"")));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_resolves_resources_and_output_from_the_source_parent() {
    let project = TestProject::new("");
    fs::create_dir(project.root().join("talk")).unwrap();
    fs::write(project.root().join("café plot.svg"), b"<svg/>").unwrap();
    fs::write(
        project.root().join("talk/deck.md"),
        "## Resources\n\n![Direct](../caf%C3%A9%20plot.svg?raw=1#circle)\n\n![Reference][plot]\n\n[Download](../caf%C3%A9%20plot.svg)\n\n[plot]: ../caf%C3%A9%20plot.svg\n\n[Site](https://example.com/) and [slide](#resources).\n\n![Remote](https://example.com/plot.svg)\n",
    ).unwrap();
    let output = project.run(&["render", "talk/deck.md", "-o", "deck"]);
    assert!(output.status.success(), "{output:?}");
    let destination = project.root().join("talk/deck");
    let html = fs::read_to_string(destination.join("index.html")).unwrap();
    assert!(html.contains("<title>deck</title>"));
    assert!(html.contains("?raw=1#circle"));
    assert!(html.contains("href=\"https://example.com/\""));
    assert!(html.contains("href=\"#resources\""));
    assert!(html.contains("src=\"https://example.com/plot.svg\""));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["files"].as_array().unwrap().len(), 2);
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_disabled_cells_resolves_defaults_and_visibility() {
    let project = TestProject::new(
        "---\ntitle: 'Source & code'\nexecute:\n  eval: false\n  echo: false\n---\n\n## Code\n\n```{r}\nhidden_default()\n```\n\n```{r}\n#| echo: true\nvisible()\n```\n\n```{r}\n#| echo: true\n#| include: false\nhidden_include()\n```\n",
    );
    let output = project.run(&["render", "slides.qmd"]);
    assert!(output.status.success(), "{output:?}");
    let html = fs::read_to_string(project.root().join("slides_html/index.html")).unwrap();
    assert!(html.contains("<title>Source &amp; code</title>"));
    assert!(html.contains("visible()"));
    assert!(!html.contains("hidden_default()"));
    assert!(!html.contains("hidden_include()"));
    assert!(!html.contains("#|"));
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_boolean_options_validate_defaults_and_nested_overrides() {
    let project = TestProject::new("");
    for value in ["false", "False", "FALSE"] {
        project.write_source(&format!(
            "---\nexecute:\n  eval: true\n---\n\n## Nested\n\n> ```{{r}}\n> #| eval: {value}\n> nested()\n> ```\n"
        ));
        let output = project.run(&["render", "slides.qmd"]);
        assert!(output.status.success(), "{output:?}");
        let html = fs::read_to_string(project.root().join("slides_html/index.html")).unwrap();
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("nested()"));
    }
    for name in ["eval", "echo", "include"] {
        for value in ["'false'", "no", "0", "null", "[false]"] {
            project.write_source(&format!(
                "---\nexecute:\n  {name}: {value}\n---\n\n```{{r}}\n#| eval: false\n#| echo: true\n#| include: true\n1\n```\n"
            ));
            let output = project.run(&["render", "slides.qmd"]);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stderr.contains("option.invalid-value"), "{output:?}");
            assert!(
                !output.stderr.contains("execution-unavailable"),
                "{output:?}"
            );
        }
    }
    project.write_source("```{r}\n#| eval: 'false'\n1\n```\n");
    let output = project.run(&["render", "slides.qmd"]);
    assert!(output.stderr.contains("option.invalid-value"), "{output:?}");
    assert!(
        !output.stderr.contains("execution-unavailable"),
        "{output:?}"
    );
    project.write_source("---\nexecute:\n  eval: false\n---\n\n```{r}\n#| eval: true\n1\n```\n");
    let output = project.run(&["render", "slides.qmd"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        output.stderr.contains("render.execution-unavailable"),
        "{output:?}"
    );
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_refuses_to_replace_directories_containing_its_inputs() {
    let project = TestProject::new("## Original\n");
    assert!(project.run(&["render", "slides.qmd"]).status.success());
    let destination = project.root().join("slides_html");
    let original = fs::read(destination.join("index.html")).unwrap();
    fs::write(destination.join("source.qmd"), "## Source inside output\n").unwrap();
    let output = project.run(&[
        "render",
        "slides_html/source.qmd",
        "-o",
        destination.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.contains("contains an input"), "{output:?}");
    assert!(destination.join("source.qmd").is_file());
    assert_eq!(fs::read(destination.join("index.html")).unwrap(), original);

    fs::write(destination.join("plot.svg"), b"<svg/>").unwrap();
    project.write_source("## Resource inside output\n\n![Plot](slides_html/plot.svg)\n");
    let output = project.run(&["render", "slides.qmd"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.contains("contains an input"), "{output:?}");
    assert_eq!(fs::read(destination.join("plot.svg")).unwrap(), b"<svg/>");
    assert_eq!(fs::read(destination.join("index.html")).unwrap(), original);
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn render_failures_preserve_the_previous_deck_and_report_source_locations() {
    let project = TestProject::new("## Original\n\nPreserve this deck.\n");
    assert!(project.run(&["render", "slides.qmd"]).status.success());
    let destination = project.root().join("slides_html");
    let old_html = fs::read(destination.join("index.html")).unwrap();
    let old_manifest = fs::read(destination.join("manifest.json")).unwrap();
    for (source, code) in [
        (fixture("malformed-frontmatter.qmd"), "syntax.invalid-yaml"),
        (fixture("duplicate-labels.qmd"), "semantic.duplicate-label"),
        (fixture("unknown-options.qmd"), "option.unknown"),
        (
            "## Image\n\n![Missing](missing.svg)\n".into(),
            "render.resource-unavailable",
        ),
        (
            "```{r}\n#| include: false\nstop('must not run')\n```\n".into(),
            "render.execution-unavailable",
        ),
        (
            "```{r}\n#| eval: 'false'\n1\n```\n".into(),
            "option.invalid-value",
        ),
        (
            "## Footnote\n\nSee[^note].\n\n[^note]: Unsupported.\n".into(),
            "render.unsupported-html",
        ),
    ] {
        project.write_source(&source);
        let output = project.run(&["render", "slides.qmd"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(output.stderr.contains("slides.qmd:"), "{output:?}");
        assert!(
            output.stderr.contains(&format!("error[{code}]")),
            "{output:?}"
        );
        assert_eq!(fs::read(destination.join("index.html")).unwrap(), old_html);
        assert_eq!(
            fs::read(destination.join("manifest.json")).unwrap(),
            old_manifest
        );
    }
    project.write_source("## Revised\n\nA replacement.\n");
    assert!(project.run(&["render", "slides.qmd"]).status.success());
    assert_ne!(fs::read(destination.join("index.html")).unwrap(), old_html);
}

#[test]
fn render_rejects_bad_arguments_and_unreadable_source() {
    let project = TestProject::new("");
    for args in [vec!["render"], vec!["render", "slides.qmd", "--to", "pdf"]] {
        let output = project.run(&args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
    let output = project.run(&["render", "missing.qmd"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.contains("missing.qmd"), "{output:?}");
    assert!(!project.root().join("slides_html").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn render_never_starts_processes() {
    let project = static_project();
    for (source, expected_code) in [
        (fixture("static-deck.qmd"), 0),
        (fixture("source-content.qmd"), 0),
        (fixture("malformed-options.qmd"), 1),
        ("```{r}\nstop('must not run')\n```\n".into(), 1),
    ] {
        project.write_source(&source);
        let (output, launches) = super::common::process::trace(
            &project,
            env!("CARGO_BIN_EXE_tractate"),
            &["render", "slides.qmd", "--to", "html"],
        );
        assert_eq!(output.status.code(), Some(expected_code), "{output:?}");
        assert!(
            launches.is_empty(),
            "render started processes: {launches:?}"
        );
    }
}
