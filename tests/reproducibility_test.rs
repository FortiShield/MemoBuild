use memobuild::docker::{dag::build_graph_from_instructions, parser::parse_dockerfile};
use tempfile::tempdir;

#[test]
fn mutable_and_external_images_are_rejected_before_reproducibility_claims() {
    let context = tempdir().expect("create context");
    let parsed = parse_dockerfile("FROM alpine:latest\n").unwrap();
    let error = build_graph_from_instructions(parsed, context.path().to_path_buf()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Only `FROM scratch` is accepted"),
        "{error:#}"
    );
}

#[test]
fn accepted_scratch_declaration_has_stable_compiler_identity() {
    let context = tempdir().expect("create context");
    let parsed = parse_dockerfile("FROM scratch\n").unwrap();
    let graph = build_graph_from_instructions(parsed, context.path().to_path_buf()).unwrap();
    let key1 = graph.nodes[0].compute_node_key(&[], None, None);
    let key2 = graph.nodes[0].compute_node_key(&[], None, None);
    assert_eq!(key1, key2);
}
