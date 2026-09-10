use crate::common::{TestProject, fixture};

#[test]
fn inspect_reports_semantic_errors_at_source_locations_without_processes() {
    for (name, code, count) in [
        ("unknown-options.qmd", "option.unknown", 2),
        ("duplicate-labels.qmd", "semantic.duplicate-label", 1),
    ] {
        let project = TestProject::new(&fixture(name));
        #[cfg(target_os = "linux")]
        let output = {
            let (output, launches) = crate::common::process::trace(
                &project,
                env!("CARGO_BIN_EXE_tractate"),
                &["inspect", "slides.qmd"],
            );
            assert!(launches.is_empty(), "{launches:?}");
            output
        };
        #[cfg(not(target_os = "linux"))]
        let output = project.inspect();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.contains("parse errors: 0\n"));
        assert!(
            output
                .stdout
                .contains(&format!("semantic errors: {count}\n"))
        );
        assert!(
            output.stderr.contains(&format!("error[{code}]")),
            "{output:?}"
        );
        if name == "unknown-options.qmd" {
            assert!(output.stderr.contains("slides.qmd:3:3"), "{output:?}");
            assert!(output.stderr.contains("slides.qmd:10:4"), "{output:?}");
        } else {
            assert!(
                output.stderr.contains("note: first declaration"),
                "{output:?}"
            );
        }
    }
}
