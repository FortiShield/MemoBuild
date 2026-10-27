use memobuild::docker::dag::{build_graph_from_instructions, compile_dockerfile};
use memobuild::docker::parser::{parse_dockerfile, Instruction};
use memobuild::graph::NodeKind;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn compile(text: &str, context: &Path) -> anyhow::Result<memobuild::graph::BuildGraph> {
    build_graph_from_instructions(parse_dockerfile(text)?, context.to_path_buf())
}

#[test]
fn scratch_base_compiles_to_an_empty_copy_plan() {
    let context = TempDir::new().expect("create context");
    let plan = compile_dockerfile(
        parse_dockerfile("# comment\nFROM scratch\n").unwrap(),
        context.path().to_path_buf(),
    )
    .expect("compile scratch");
    assert_eq!(plan.graph.nodes.len(), 1);
    assert!(matches!(plan.graph.nodes[0].kind, NodeKind::From));
    assert!(plan.copies.is_empty());
    assert!(plan.cmd.is_none());
    assert!(plan.entrypoint.is_none());
}

#[test]
fn unknown_instruction_is_deterministic_and_reports_source_line() {
    let first = parse_dockerfile("# header\nFUTURE_BUILD value").unwrap_err();
    let second = parse_dockerfile("# header\nFUTURE_BUILD value").unwrap_err();
    assert_eq!(first.to_string(), second.to_string());
    assert_eq!(
        first.to_string(),
        "Dockerfile line 2: unknown instruction 'FUTURE_BUILD'; remove it or use an instruction supported by MemoBuild"
    );
}

#[test]
fn recognized_but_unimplemented_instruction_has_distinct_error() {
    let context = TempDir::new().expect("create context");
    let error = compile("FROM scratch\nEXPOSE 8080\n", context.path()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("line 2"), "{message}");
    assert!(
        message.contains("EXPOSE is recognized but unsupported"),
        "{message}"
    );
}

#[test]
fn every_recognized_non_copy_instruction_is_rejected_by_the_compiler() {
    let context = TempDir::new().expect("create context");
    let unsupported = [
        ("WORKDIR", "WORKDIR /app"),
        ("RUN", "RUN echo build"),
        ("ENV", "ENV APP_MODE=prod"),
        ("ADD", "ADD source.tar /app"),
        ("ARG", "ARG BUILD_MODE=prod"),
        ("USER", "USER nobody"),
        ("SHELL", "SHELL [\"sh\", \"-c\"]"),
        ("LABEL", "LABEL org.example.name=memo"),
        ("EXPOSE", "EXPOSE 8080"),
        ("VOLUME", "VOLUME /data"),
        ("HEALTHCHECK", "HEALTHCHECK CMD true"),
        ("STOPSIGNAL", "STOPSIGNAL SIGTERM"),
        ("ONBUILD", "ONBUILD RUN echo child"),
        ("MAINTAINER", "MAINTAINER MemoBuild"),
        ("GIT", "GIT https://example.invalid/repo"),
        ("RUN_EXTEND", "RUN_EXTEND echo build"),
        ("COPY_EXTEND", "COPY_EXTEND source destination"),
        ("HOOK", "HOOK before-build"),
    ];

    for (keyword, instruction) in unsupported {
        let dockerfile = format!("FROM scratch\n{instruction}\n");
        let error = compile(&dockerfile, context.path()).unwrap_err();
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains(&format!("line 2: {keyword} is recognized but unsupported")),
            "{keyword} produced an unexpected diagnostic: {error:#}"
        );
    }
}

#[test]
fn malformed_arguments_and_multiline_syntax_fail_during_parsing() {
    for dockerfile in [
        "FROM scratch\nRUN\n",
        "FROM scratch\nCOPY source\n",
        "FROM scratch\nENV 1INVALID=value\n",
        "FROM scratch\nCMD [\"unterminated]\n",
        "FROM scratch \\\n AS base\n",
        "FROM scratch\n# escape=`\n",
    ] {
        assert!(
            parse_dockerfile(dockerfile).is_err(),
            "expected parse error for {dockerfile:?}"
        );
    }
}

#[test]
fn only_json_array_command_metadata_is_supported() {
    let context = TempDir::new().expect("create context");
    for form in ["CMD /app --flag", "ENTRYPOINT /entrypoint"] {
        let dockerfile = format!("FROM scratch\n{form}\n");
        let error = parse_dockerfile(&dockerfile).unwrap_err();
        assert!(
            error.to_string().contains("shell-form"),
            "unexpected diagnostic: {error:#}"
        );
    }

    let plan = compile_dockerfile(
        parse_dockerfile(
            "FROM scratch\nENTRYPOINT [\"/entry point\", \"--mode\", \"two words\"]\n\
             CMD [\"argument with spaces\", \"tail\"]\n",
        )
        .unwrap(),
        context.path().to_path_buf(),
    )
    .unwrap();
    assert_eq!(
        plan.entrypoint,
        Some(vec![
            "/entry point".to_string(),
            "--mode".to_string(),
            "two words".to_string()
        ])
    );
    assert_eq!(
        plan.cmd,
        Some(vec!["argument with spaces".to_string(), "tail".to_string()])
    );
}

#[test]
fn parser_preserves_shell_quoting_and_source_locations() {
    let parsed = parse_dockerfile("# comment\n  RUN printf 'hello world'\n").unwrap();
    assert_eq!(parsed[0].location.line, 2);
    assert_eq!(parsed[0].location.column, 3);
    assert_eq!(
        parsed[0].instruction,
        Instruction::Run("printf 'hello world'".to_string())
    );
}

#[test]
fn external_base_forms_are_rejected_by_the_explicit_scratch_only_policy() {
    let context = TempDir::new().expect("create context");
    for base in [
        "FROM alpine:latest",
        "FROM registry.example/team/image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "FROM local-image",
    ] {
        let error = compile(base, context.path()).unwrap_err();
        assert!(
            error.to_string().contains("Only `FROM scratch` is accepted"),
            "unexpected base policy diagnostic for {base:?}: {error:#}"
        );
    }
}

#[test]
fn aliases_and_multistage_references_are_rejected() {
    let context = TempDir::new().expect("create context");
    let alias = compile("FROM scratch AS builder\n", context.path()).unwrap_err();
    assert!(alias.to_string().contains("stage aliases are unsupported"));

    let stage_ref = compile(
        "FROM scratch\nCOPY --from=builder /artifact /artifact\n",
        context.path(),
    )
    .unwrap_err();
    assert!(
        stage_ref
            .to_string()
            .contains("COPY references unknown stage 'builder'"),
        "{stage_ref:#}"
    );

    let second_stage = compile("FROM scratch\nFROM scratch\n", context.path()).unwrap_err();
    assert!(second_stage
        .to_string()
        .contains("multi-stage builds are unsupported"));
}

#[test]
fn copy_paths_must_exist_and_remain_inside_context() {
    let context = TempDir::new().expect("create context");
    let traversal =
        compile("FROM scratch\nCOPY ../outside /artifact\n", context.path()).unwrap_err();
    assert!(
        traversal
            .to_string()
            .contains("must be a relative path inside the build context"),
        "{traversal:#}"
    );

    let missing = compile("FROM scratch\nCOPY absent.txt /artifact\n", context.path()).unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("COPY source 'absent.txt' does not exist"),
        "{missing:#}"
    );
}

#[test]
fn existing_copy_source_compiles_to_a_context_relative_operation() {
    let context = TempDir::new().expect("create context");
    std::fs::write(context.path().join("source.txt"), "fixture").expect("write source fixture");
    let plan = compile_dockerfile(
        parse_dockerfile("FROM scratch\nCOPY source.txt /artifact\n").unwrap(),
        context.path().to_path_buf(),
    )
    .expect("compile supported COPY");
    assert_eq!(plan.copies.len(), 1);
    assert_eq!(plan.copies[0].source, PathBuf::from("source.txt"));
    assert_eq!(plan.copies[0].destination, PathBuf::from("artifact"));
}

#[cfg(unix)]
#[test]
fn copy_symlink_cannot_resolve_outside_context() {
    use std::os::unix::fs::symlink;

    let context = TempDir::new().expect("create context");
    let outside = TempDir::new().expect("create outside fixture");
    std::fs::write(outside.path().join("secret"), "fixture").expect("write fixture");
    symlink(outside.path().join("secret"), context.path().join("escape"))
        .expect("create escape symlink");
    let error = compile("FROM scratch\nCOPY escape /artifact\n", context.path()).unwrap_err();
    assert!(error.to_string().contains("symlink"), "{error:#}");
}

#[test]
fn production_cli_rejects_run_before_cache_or_host_execution() {
    let context = TempDir::new().expect("create context");
    let cache = TempDir::new().expect("create cache parent");
    let cache_dir = cache.path().join("must-not-be-created");
    let marker = context.path().join("host-marker");
    std::fs::write(
        context.path().join("Dockerfile"),
        "FROM scratch\nRUN touch host-marker\n",
    )
    .expect("write Dockerfile");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_memobuild"))
        .args(["build", "--file", "Dockerfile"])
        .current_dir(context.path())
        .env("MEMOBUILD_CACHE_DIR", &cache_dir)
        .output()
        .expect("run CLI");
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{diagnostic}");
    assert!(
        diagnostic.contains("RUN is recognized but unsupported"),
        "{diagnostic}"
    );
    assert!(!marker.exists(), "unsupported RUN executed on the host");
    assert!(
        !cache_dir.exists(),
        "validation failure must happen before cache initialization"
    );
}

#[test]
fn custom_memo_instructions_remain_recognized_but_are_not_silently_executed() {
    let context = TempDir::new().expect("create context");
    let parsed = parse_dockerfile("FROM scratch\nRUN_EXTEND echo test\n").unwrap();
    assert!(matches!(
        parsed[1].instruction,
        Instruction::Unsupported { ref keyword, .. } if keyword == "RUN_EXTEND"
    ));
    let error = build_graph_from_instructions(parsed, PathBuf::from(context.path())).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("RUN_EXTEND is recognized but unsupported"),
        "{error:#}"
    );
}
