use tractate::{Compiler, CompilerSnapshot, DiagnosticCode, EvaluationSession, Origin, SourceFile};

fn snapshot(source: &str) -> CompilerSnapshot {
    Compiler::new(SourceFile::new("slides.qmd", source)).snapshot()
}

#[test]
fn evaluation_plan_lowers_source_cells_including_nested_containers() {
    let source = "Leading prose.\n\n```r\nordinary()\n```\n\n```{r}\n#| label: first\nx <- 1\n```\n\n## Slide\n\n> ```{r}\n> #| label: nested\n> print(x)\n> ```\n\n::: {.group}\n\n```{r}\n#| label: div\nplot(x)\n```\n\n:::\n";
    let snapshot = snapshot(source);
    let plan = snapshot.evaluation_plan().unwrap();
    assert_eq!(plan.source_revision, snapshot.revision());
    assert_eq!(plan.cells.len(), 3);
    assert_eq!(
        plan.cells
            .iter()
            .map(|cell| cell.code.as_str())
            .collect::<Vec<_>>(),
        ["x <- 1\n", "print(x)\n", "plot(x)\n"]
    );
    for cell in &plan.cells {
        assert_eq!(cell.language.as_deref(), Some("r"));
        assert_eq!(cell.session, EvaluationSession::Default);
        assert_eq!(cell.origin.source_span().file(), snapshot.source());
        assert!(cell.origin.source_span().text().contains("```{r}"));
    }
    assert_eq!(plan.cells[0].predecessor, None);
    assert_eq!(plan.cells[1].predecessor, Some(plan.cells[0].cell_id));
    assert_eq!(plan.cells[2].predecessor, Some(plan.cells[1].cell_id));
}

#[test]
fn evaluation_plan_resolves_eval_defaults_and_local_boolean_overrides() {
    for disabled in ["false", "False", "FALSE"] {
        for enabled in ["true", "True", "TRUE"] {
            let source = format!(
                "---\nexecute:\n  eval: {disabled}\n---\n\n```{{r}}\ninherited()\n```\n\n> ```{{r}}\n> #| eval: {enabled}\n> enabled()\n> ```\n\n```{{r}}\n#| eval: {disabled}\ndisabled()\n```\n"
            );
            let snapshot = snapshot(&source);
            let plan = snapshot.evaluation_plan().unwrap();
            assert_eq!(plan.cells.len(), 1);
            assert_eq!(plan.cells[0].code, "enabled()\n");
            assert_eq!(plan.cells[0].predecessor, None);
            assert_eq!(snapshot.inspection().summary.executable_cells, 3);
        }
    }
    for source in [
        "",
        "## Prose\n\nText.\n",
        "```r\nordinary()\n```\n",
        "```{r}\n#| eval: false\ndisabled()\n```\n",
    ] {
        assert!(snapshot(source).evaluation_plan().unwrap().cells.is_empty());
    }
}

#[test]
fn evaluation_plan_excludes_disabled_cells_from_stateful_prefixes() {
    let original = "```{r}\n#| label: a\na()\n```\n\n```{r}\n#| label: b\n#| eval: false\nb()\n```\n\n```{r}\n#| label: c\nc()\n```\n\n```{r}\n#| label: d\nd()\n```\n";
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", original));
    let retained = compiler.snapshot();
    let original_plan = retained.evaluation_plan().unwrap();
    assert_eq!(original_plan.cells.len(), 3);
    assert_eq!(
        original_plan.cells[1].predecessor,
        Some(original_plan.cells[0].cell_id)
    );
    assert_eq!(
        original_plan.cells[2].predecessor,
        Some(original_plan.cells[1].cell_id)
    );

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        original.replace("b()", "changed_disabled()"),
    ));
    let edited = compiler.snapshot();
    assert_eq!(observation(&edited), observation(&retained));

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        original.replace("eval: false", "eval: true"),
    ));
    let enabled = compiler.snapshot();
    let enabled_plan = enabled.evaluation_plan().unwrap();
    assert_eq!(enabled_plan.cells.len(), 4);
    assert_eq!(
        enabled_plan.cells[2].predecessor,
        Some(enabled_plan.cells[1].cell_id)
    );
    assert_eq!(enabled_plan.cells[1].code, "b()\n");

    compiler.update_source(SourceFile::new("slides.qmd", original));
    assert_eq!(observation(&compiler.snapshot()), observation(&retained));
    assert_eq!(retained.evaluation_plan().unwrap(), original_plan);
}

#[test]
fn evaluation_plan_keeps_session_and_language_prefixes_independent() {
    let source = "---\nexecute:\n  session: analysis\n---\n\n```{r}\na()\n```\n\n```{r}\n#| session: default\nb()\n```\n\n```{r}\n#| session: isolated\nc()\n```\n\n```{r}\n#| eval: false\ndisabled()\n```\n\n```{python}\nother_language()\n```\n\n```{r}\nd()\n```\n\n```{r}\n#| session: default\ne()\n```\n\n```{r}\n#| session: isolated\nf()\n```\n";
    let snapshot = snapshot(source);
    let plan = snapshot.evaluation_plan().unwrap();
    let cells = &plan.cells;
    assert_eq!(cells.len(), 7);
    assert_eq!(
        cells[0].session,
        EvaluationSession::Named("analysis".into())
    );
    assert_eq!(cells[1].session, EvaluationSession::Default);
    assert_eq!(cells[2].session, EvaluationSession::Isolated);
    assert_eq!(cells[3].language.as_deref(), Some("python"));
    for cell in &cells[..4] {
        assert_eq!(cell.predecessor, None);
    }
    assert_eq!(cells[4].predecessor, Some(cells[0].cell_id));
    assert_eq!(cells[5].predecessor, Some(cells[1].cell_id));
    assert_eq!(cells[6].session, EvaluationSession::Isolated);
    assert_eq!(cells[6].predecessor, None);
}

#[test]
fn evaluation_plan_resolves_session_names_as_yaml_strings() {
    for value in [
        "analysis",
        "'analysis'",
        "\"analysis\"",
        "|-\n#|   analysis",
        ">-\n#|   analysis",
        "|2-\n#|   analysis",
    ] {
        let source = format!("```{{r}}\n#| session: {value}\n1\n```\n");
        let snapshot = snapshot(&source);
        assert_eq!(
            snapshot.evaluation_plan().unwrap().cells[0].session,
            EvaluationSession::Named("analysis".into())
        );
    }
    for value in [
        "analysis",
        "'analysis'",
        "|-\n    analysis",
        "|2-\n    analysis",
        ">2-\n    analysis",
    ] {
        let source = format!("---\nexecute:\n  session: {value}\n---\n\n```{{r}}\n1\n```\n");
        let snapshot = snapshot(&source);
        assert_eq!(
            snapshot.evaluation_plan().unwrap().cells[0].session,
            EvaluationSession::Named("analysis".into())
        );
    }
    let snapshot = snapshot("```{r}\n#| session: 'Default'\n1\n```\n");
    assert_eq!(
        snapshot.evaluation_plan().unwrap().cells[0].session,
        EvaluationSession::Named("Default".into())
    );
}

#[test]
fn evaluation_plan_ignores_render_options_and_performs_no_file_io() {
    let directory = tempfile::tempdir().unwrap();
    let source = "---\ntitle: Title\nexecute:\n  echo: false\n  include: false\n  results: hide\n---\n\n```{r}\n#| inputs: [missing.csv]\n#| cache: false\nstop('must never run during planning')\n```\n\n```{=typst}\nBackend-specific content.\n```\n\n![Missing](missing.svg)\n";
    let snapshot = Compiler::new(SourceFile::new(
        directory.path().join("missing/deck.qmd"),
        source,
    ))
    .snapshot();
    let plan = snapshot.evaluation_plan().unwrap();
    assert_eq!(plan.cells.len(), 1);
    assert_eq!(
        plan.cells[0].code,
        "stop('must never run during planning')\n"
    );
    assert!(std::ptr::eq(plan, snapshot.evaluation_plan().unwrap()));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);

    let base = "```{r}\n1 + 1\n```\n";
    for options in [
        "echo: false",
        "include: false",
        "results: hide",
        "fig-cap: A caption",
        "echo: 'invalid'",
    ] {
        let edited = base.replace("1 + 1", &format!("#| {options}\n1 + 1"));
        assert_eq!(
            snapshot_observation(base),
            snapshot_observation(&edited),
            "{options}"
        );
    }
}

#[test]
fn evaluation_plan_rejects_invalid_eval_and_session_declarations_before_planning() {
    for (key, values) in [
        ("eval", vec!["'false'", "yes", "0", "null", "[]", "{}", ""]),
        (
            "session",
            vec!["false", "0", "null", "''", "'two names'", "[]", "{}", ""],
        ),
    ] {
        for value in values {
            for source in [
                format!("```{{r}}\n#| {key}: {value}\n1\n```\n"),
                format!(
                    "---\nexecute:\n  {key}: {value}\n---\n\n```{{r}}\n#| eval: false\n#| session: default\n1\n```\n"
                ),
                format!("---\nexecute:\n  {key}: {value}\n---\n"),
            ] {
                let snapshot = snapshot(&source);
                let diagnostics = snapshot.evaluation_plan().unwrap_err();
                assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
                assert_eq!(
                    diagnostics[0].code,
                    DiagnosticCode::InvalidOptionValue,
                    "{source}"
                );
                assert_eq!(
                    diagnostics[0].primary.source_span().file(),
                    snapshot.source()
                );
                assert!(diagnostics[0].message.contains(key));
            }
        }
    }
    for source in [
        include_str!("fixtures/malformed-options.qmd"),
        include_str!("fixtures/unknown-options.qmd"),
        include_str!("fixtures/duplicate-labels.qmd"),
    ] {
        let snapshot = snapshot(source);
        assert_eq!(
            snapshot.evaluation_plan().unwrap_err(),
            snapshot.inspection().diagnostics
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CellObservation {
    code: String,
    language: Option<String>,
    session: EvaluationSession,
    predecessor: Option<usize>,
}

fn observation(snapshot: &CompilerSnapshot) -> Vec<CellObservation> {
    let plan = snapshot.evaluation_plan().unwrap();
    assert_eq!(plan.source_revision, snapshot.revision());
    plan.cells
        .iter()
        .map(|cell| CellObservation {
            code: cell.code.clone(),
            language: cell.language.clone(),
            session: cell.session.clone(),
            predecessor: cell.predecessor.map(|predecessor| {
                plan.cells
                    .iter()
                    .position(|cell| cell.cell_id == predecessor)
                    .unwrap()
            }),
        })
        .collect()
}

fn snapshot_observation(source: &str) -> Vec<CellObservation> {
    observation(&snapshot(source))
}

#[test]
fn evaluation_plan_incremental_edits_match_clean_full_builds() {
    let initial = "---\nexecute:\n  eval: true\n---\n\n## First\n\n```{r}\n#| label: a\na()\n```\n\n```{r}\n#| label: b\n#| eval: false\nb()\n```\n\n## Second\n\n```{r}\n#| label: c\nc()\n```\n";
    for line_ending in ["\n", "\r\n"] {
        let (first, second) = initial.split_once("## Second").unwrap();
        let moved = format!("{second}\n{first}");
        let mut compiler = Compiler::new(SourceFile::new(
            "slides.qmd",
            initial.replace('\n', line_ending),
        ));
        for source in [
            initial.to_owned(),
            initial.replace("## First", "Leading café.\n\n## Renamed"),
            initial.replace("eval: false", "eval: true"),
            initial.replace("a()", "changed()"),
            initial.replace("b()", "disabled_edit()"),
            initial.replace("#| label: c", "#| label: c\n#| session: isolated"),
            initial.replace("execute:\n  eval: true", "execute:\n  eval: false"),
            initial.replace("a()", "a()\n```\n\n```{r}\ninserted()"),
            moved,
            initial.replace("#| eval: false", "#| eval: 'false'"),
            initial.replace("#| label: b", "#| label: a"),
            String::new(),
            initial.to_owned(),
        ] {
            let source = source.replace('\n', line_ending);
            compiler.update_source(SourceFile::new("slides.qmd", &source));
            let current = compiler.snapshot();
            let fresh = Compiler::new(current.source().clone()).snapshot();
            match (current.evaluation_plan(), fresh.evaluation_plan()) {
                (Ok(current_plan), Ok(fresh_plan)) => {
                    assert_eq!(observation(&current), observation(&fresh));
                    assert_eq!(
                        current_plan
                            .cells
                            .iter()
                            .map(|cell| &cell.origin)
                            .collect::<Vec<&Origin>>(),
                        fresh_plan
                            .cells
                            .iter()
                            .map(|cell| &cell.origin)
                            .collect::<Vec<_>>()
                    );
                }
                (Err(current), Err(fresh)) => assert_eq!(current, fresh),
                results => panic!("incremental and fresh plans differ: {results:?}"),
            }
        }
    }
}
