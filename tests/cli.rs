use std::process::Command;

#[test]
fn inspect_reports_document_structure() {
    let output = Command::new(env!("CARGO_BIN_EXE_tractate"))
        .args(["inspect", "tests/fixtures/minimal.qmd"])
        .output()
        .expect("run tractate");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(stdout.contains("headings: 1"));
    assert!(stdout.contains("code blocks: 2"));
    assert!(stdout.contains("executable cells: 1"));
    assert!(stdout.contains("executable languages: r"));
    assert!(stdout.contains("parse errors: 0"));
}
