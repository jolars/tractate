mod common;

use common::{Edit, TestProject, assert_matches_full_build, fixture};

#[test]
fn inspect_reports_document_structure() {
    let project = TestProject::new(&fixture("minimal.qmd"));
    let output = project.inspect();

    assert!(output.status.success(), "stderr: {}", output.stderr);
    let stdout = output.stdout;
    assert!(stdout.contains("headings: 1"));
    assert!(stdout.contains("code blocks: 2"));
    assert!(stdout.contains("executable cells: 1"));
    assert!(stdout.contains("executable languages: r"));
    assert!(stdout.contains("parse errors: 0"));
}

#[test]
fn inspect_rejects_malformed_input() {
    for name in ["malformed-frontmatter.qmd", "malformed-options.qmd"] {
        let project = TestProject::new(&fixture(name));
        let output = project.inspect();

        assert_eq!(output.status.code(), Some(1), "{name}: {output:?}");
        assert!(output.stdout.contains("executable cells: 1"), "{output:?}");
        assert!(output.stdout.contains("parse errors: 1"), "{output:?}");
        assert_eq!(
            output.stderr, "tractate: parser reported 1 error(s) in `slides.qmd`\n",
            "{name}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn inspect_never_starts_processes() {
    for name in [
        "minimal.qmd",
        "malformed-frontmatter.qmd",
        "malformed-options.qmd",
    ] {
        let project = TestProject::new(&fixture(name));
        let (output, launches) = common::process::trace(
            &project,
            env!("CARGO_BIN_EXE_tractate"),
            &["inspect", "slides.qmd"],
        );

        let expected_code = if name == "minimal.qmd" { 0 } else { 1 };
        assert_eq!(output.status.code(), Some(expected_code), "{output:?}");
        assert!(
            launches.is_empty(),
            "inspect started a process for {name}:\n{}",
            launches.join("\n")
        );
    }
}

#[test]
fn inspect_edit_sequence_matches_clean_full_builds() {
    let initial = fixture("minimal.qmd");
    let project = TestProject::new(&initial);
    let edits = [
        Edit {
            name: "insert prose with Unicode",
            old: "## Simulation",
            new: "Leading prose with café and α.\n\n## Simulation",
        },
        Edit {
            name: "add a slide",
            old: "plot(x)\n```",
            new: "plot(x)\n```\n\n## Results\n\nA result.",
        },
        Edit {
            name: "make display code executable",
            old: "```r\nx <- 1",
            new: "```{r}\nx <- 1",
        },
        Edit {
            name: "break metadata",
            old: "title: Minimal presentation",
            new: "title: [",
        },
        Edit {
            name: "repair metadata",
            old: "title: [",
            new: "title: Repaired presentation",
        },
        Edit {
            name: "remove the added slide",
            old: "\n\n## Results\n\nA result.",
            new: "",
        },
    ];

    assert_matches_full_build(
        &initial,
        &edits,
        |source| {
            project.write_source(source);
            project.inspect()
        },
        |source| TestProject::new(source).inspect(),
    );
}
