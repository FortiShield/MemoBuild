use memobuild::{docker, graph::NodeKind};
use tempfile::tempdir;

#[test]
fn strict_compiler_builds_supported_scratch_copy_and_config_nodes() {
    let context = tempdir().expect("create context");
    std::fs::write(context.path().join("hello.txt"), "hello").expect("write context fixture");
    let parsed = docker::parser::parse_dockerfile(
        "# scratch copy fixture\nFROM scratch\nCOPY hello.txt /hello.txt\nCMD [\"/hello.txt\"]\n",
    )
    .unwrap();
    let graph =
        docker::dag::build_graph_from_instructions(parsed, context.path().to_path_buf()).unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert!(matches!(graph.nodes[0].kind, NodeKind::From));
    assert!(graph.nodes[0].deps.is_empty());
    assert!(matches!(graph.nodes[1].kind, NodeKind::Copy { .. }));
    assert!(matches!(graph.nodes[2].kind, NodeKind::Cmd));
}

#[test]
fn known_dockerfile_examples_fail_when_their_semantics_are_not_implemented() {
    let context = tempdir().expect("create context");
    let parsed = docker::parser::parse_dockerfile(
        "FROM golang:1.21-alpine AS builder\nWORKDIR /app\nRUN go build\n",
    )
    .unwrap();
    let error = docker::dag::build_graph_from_instructions(parsed, context.path().to_path_buf())
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Only `FROM scratch` is accepted"));
}

#[test]
fn unsupported_metadata_is_not_reported_as_a_successful_build_node() {
    let context = tempdir().expect("create context");
    let parsed = docker::parser::parse_dockerfile("FROM scratch\nWORKDIR /app\n").unwrap();
    let error = docker::dag::build_graph_from_instructions(parsed, context.path().to_path_buf())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("WORKDIR is recognized but unsupported"),
        "{error:#}"
    );
}

#[test]
fn command_metadata_preserves_json_argument_boundaries() {
    let context = tempdir().expect("create context");
    let plan = docker::dag::compile_dockerfile(
        docker::parser::parse_dockerfile(
            "FROM scratch\nENTRYPOINT [\"/entry\", \"two words\"]\nCMD [\"--name\", \"value\"]\n",
        )
        .unwrap(),
        context.path().to_path_buf(),
    )
    .unwrap();
    assert_eq!(
        plan.entrypoint,
        Some(vec!["/entry".to_string(), "two words".to_string()])
    );
    assert_eq!(
        plan.cmd,
        Some(vec!["--name".to_string(), "value".to_string()])
    );
}
