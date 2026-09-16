use std::fs;

use super::common::{TestProject, fixture};

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn no_execute_renders_documents_without_required_computation() {
    for source in [
        String::new(),
        "## Source\n\n```r\nstop('ordinary code')\n```\n".into(),
        fixture("source-content.qmd"),
        "---\nexecute:\n  eval: false\n  echo: false\n---\n\n## Cells\n\n```{r}\nhidden_default()\n```\n\n> ```{r}\n> #| echo: true\n> visible()\n> ```\n\n```{r}\n#| include: false\n#| echo: true\nhidden_include()\n```\n".into(),
        "---\nexecute:\n  eval: true\n---\n\n> ```{r}\n> #| eval: FALSE\n> visible()\n> ```\n".into(),
    ] {
        let project = TestProject::new(&source);
        let output = project.run(&["render", "slides.qmd", "--to", "html", "--no-execute"]);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(output.stdout.contains("slides_html/index.html"), "{output:?}");
        let destination = project.root().join("slides_html");
        let html = fs::read_to_string(destination.join("index.html")).unwrap();
        let manifest = fs::read(destination.join("manifest.json")).unwrap();
        assert!(html.starts_with("<!doctype html>"));
        assert!(!html.contains("hidden_default()"));
        assert!(!html.contains("hidden_include()"));
        if source.contains("visible()") {
            assert!(html.contains("visible()"));
        }

        let oracle = TestProject::new(&source);
        let output = oracle.run(&["render", "slides.qmd"]);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            html,
            fs::read_to_string(oracle.root().join("slides_html/index.html")).unwrap()
        );
        assert_eq!(
            manifest,
            fs::read(oracle.root().join("slides_html/manifest.json")).unwrap()
        );
    }
}

#[test]
fn no_execute_reports_each_required_result_at_its_source_location() {
    let project = TestProject::new(
        "## Cells\n\n```{r}\n#| eval: false\ndisabled()\n```\n\n```{r}\n#| label: first\n1\n```\n\n> ```{r}\n> #| label: second\n> 2\n> ```\n",
    );
    let output = project.run(&["render", "slides.qmd", "--no-execute"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output
            .stderr
            .matches("error[render.result-unavailable]")
            .count(),
        2,
        "{output:?}"
    );
    assert!(output.stderr.contains("slides.qmd:8:1:"), "{output:?}");
    assert!(output.stderr.contains("slides.qmd:13:3:"), "{output:?}");
    assert!(output.stderr.contains("required result"), "{output:?}");
    assert!(output.stderr.contains("--no-execute"), "{output:?}");
    assert!(!project.root().join("slides_html").exists());
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
#[test]
fn no_execute_missing_results_preserve_a_previous_deck() {
    let project = TestProject::new("## Original\n\nPreserve this deck.\n");
    assert!(project.run(&["render", "slides.qmd"]).status.success());
    let destination = project.root().join("slides_html");
    let html = fs::read(destination.join("index.html")).unwrap();
    let manifest = fs::read(destination.join("manifest.json")).unwrap();
    for options in [
        "",
        "#| echo: false\n",
        "#| include: false\n",
        "#| results: hide\n",
        "#| cache: true\n",
        "#| cache: false\n",
        "#| eval: true\n",
    ] {
        project.write_source(&format!(
            "---\nexecute:\n  eval: {}\n---\n\n```{{r}}\n{options}stop('must not run')\n```\n",
            if options.contains("eval:") {
                "false"
            } else {
                "true"
            },
        ));
        let output = project.run(&["render", "slides.qmd", "--no-execute"]);
        assert_eq!(output.status.code(), Some(1), "{options}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            output.stderr.contains("render.result-unavailable"),
            "{output:?}"
        );
        assert_eq!(fs::read(destination.join("index.html")).unwrap(), html);
        assert_eq!(
            fs::read(destination.join("manifest.json")).unwrap(),
            manifest
        );
    }
}

#[test]
fn no_execute_validates_source_before_reporting_missing_results() {
    for (source, code) in [
        (fixture("malformed-frontmatter.qmd"), "syntax.invalid-yaml"),
        (fixture("malformed-options.qmd"), "syntax.invalid-yaml"),
        (fixture("duplicate-labels.qmd"), "semantic.duplicate-label"),
        (fixture("unknown-options.qmd"), "option.unknown"),
        (
            "```{r}\n#| eval: 'false'\n1\n```\n".into(),
            "option.invalid-value",
        ),
        (
            "---\nexecute:\n  eval: 'false'\n---\n\n```{r}\n#| eval: true\n1\n```\n".into(),
            "option.invalid-value",
        ),
    ] {
        let project = TestProject::new(&source);
        let output = project.run(&["render", "slides.qmd", "--no-execute"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.contains(code), "{output:?}");
        assert!(
            !output.stderr.contains("render.result-unavailable"),
            "{output:?}"
        );
        assert!(!project.root().join("slides_html").exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn no_execute_never_starts_processes() {
    for (source, expected_code) in [
        (fixture("source-content.qmd"), 0),
        (fixture("malformed-options.qmd"), 1),
        ("```{r}\nstop('must not run')\n```\n".into(), 1),
        (
            "```{r}\n#| include: false\nstop('must not run')\n```\n".into(),
            1,
        ),
    ] {
        let project = TestProject::new(&source);
        let (output, launches) = super::common::process::trace(
            &project,
            env!("CARGO_BIN_EXE_tractate"),
            &["render", "slides.qmd", "--no-execute"],
        );
        assert_eq!(output.status.code(), Some(expected_code), "{output:?}");
        assert!(
            launches.is_empty(),
            "--no-execute started processes: {launches:?}"
        );
    }
}
