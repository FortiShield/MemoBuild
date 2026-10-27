use memobuild::docker;

fn main() -> anyhow::Result<()> {
    let dockerfile_content = "FROM scratch\n";

    println!("Strict Dockerfile validation and DAG construction");
    let instructions = docker::parser::parse_dockerfile(dockerfile_content)?;
    println!("Parsed {} instruction(s)", instructions.len());

    let plan = docker::dag::compile_dockerfile(instructions, std::env::current_dir()?)?;
    println!("Compiled {} node(s)", plan.graph.nodes.len());
    for node in &plan.graph.nodes {
        println!("  {} | Kind: {:?}", node.name, node.kind);
    }

    let key = plan.graph.nodes[0].compute_node_key(&[], None, None);
    println!("Stable compiler-node key: {key}");
    println!("COPY operations: {}", plan.copies.len());
    println!("RUN execution and cache restoration are not supported by this build path.");

    Ok(())
}
